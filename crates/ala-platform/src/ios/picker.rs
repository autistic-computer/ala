use crate::ios::{id, nil, ns_array, ns_string, nsstring_to_string};
use crate::pick::{PickSource, PickedFile};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{BOOL, Class, Object, Protocol, Sel, YES},
    sel, sel_impl,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    path::Path,
    ptr,
    sync::{Arc, Mutex},
};

const CAMERA: isize = 1;
const COMPATIBLE: isize = 2;
const JPEG_QUALITY: f64 = 0.85;
const PHOTO: &str = "Photo";

type Done = Box<dyn FnOnce(Vec<PickedFile>)>;

thread_local! {
    static PENDING: RefCell<Option<Done>> = const { RefCell::new(None) };
    static LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
    static DELEGATE: Cell<id> = const { Cell::new(ptr::null_mut()) };
}

pub fn can_pick(source: PickSource) -> bool {
    match source {
        PickSource::Camera => unsafe {
            let available: BOOL =
                msg_send![class!(UIImagePickerController), isSourceTypeAvailable: CAMERA];
            available == YES
        },
        PickSource::Photos => Class::get("PHPickerViewController").is_some(),
        PickSource::Files => Class::get("UIDocumentPickerViewController").is_some(),
    }
}

pub fn pick(source: PickSource, limit: u64, done: impl FnOnce(Vec<PickedFile>) + 'static) {
    let Some(host) = (unsafe { top_controller() }) else {
        done(Vec::new());
        return;
    };
    if let Some(previous) = PENDING.with_borrow_mut(|pending| pending.replace(Box::new(done))) {
        previous(Vec::new());
    }
    LIMIT.set(limit);
    unsafe {
        let delegate = delegate();
        let picker = match source {
            PickSource::Photos => photos(delegate),
            PickSource::Camera => camera(delegate),
            PickSource::Files => files(delegate),
        };
        if picker.is_null() {
            finish(Vec::new());
            return;
        }
        let _: () = msg_send![host, presentViewController: picker animated: YES completion: nil];
        let _: () = msg_send![picker, release];
    }
}

unsafe fn top_controller() -> Option<id> {
    let view = crate::ios::window::active_view();
    if view.is_null() {
        return None;
    }
    let window: id = msg_send![view, window];
    if window.is_null() {
        return None;
    }
    let mut controller: id = msg_send![window, rootViewController];
    while !controller.is_null() {
        let presented: id = msg_send![controller, presentedViewController];
        if presented.is_null() {
            break;
        }
        controller = presented;
    }
    (!controller.is_null()).then_some(controller)
}

unsafe fn photos(delegate: id) -> id {
    let (Some(configuration), Some(controller)) = (
        Class::get("PHPickerConfiguration"),
        Class::get("PHPickerViewController"),
    ) else {
        return nil;
    };
    let config: id = msg_send![configuration, alloc];
    let config: id = msg_send![config, init];
    let _: () = msg_send![config, setSelectionLimit: 0isize];
    let filter: id = msg_send![class!(PHPickerFilter), imagesFilter];
    let _: () = msg_send![config, setFilter: filter];
    let _: () = msg_send![config, setPreferredAssetRepresentationMode: COMPATIBLE];
    let picker: id = msg_send![controller, alloc];
    let picker: id = msg_send![picker, initWithConfiguration: config];
    let _: () = msg_send![config, release];
    let _: () = msg_send![picker, setDelegate: delegate];
    picker
}

unsafe fn camera(delegate: id) -> id {
    if !can_pick(PickSource::Camera) {
        return nil;
    }
    let picker: id = msg_send![class!(UIImagePickerController), alloc];
    let picker: id = msg_send![picker, init];
    let _: () = msg_send![picker, setSourceType: CAMERA];
    let _: () = msg_send![picker, setDelegate: delegate];
    picker
}

unsafe fn files(delegate: id) -> id {
    let Some(controller) = Class::get("UIDocumentPickerViewController") else {
        return nil;
    };
    let data: id = msg_send![class!(UTType), typeWithIdentifier: ns_string("public.data")];
    if data.is_null() {
        return nil;
    }
    let picker: id = msg_send![controller, alloc];
    let picker: id = msg_send![picker, initForOpeningContentTypes: ns_array(&[data]) asCopy: YES];
    let _: () = msg_send![picker, setAllowsMultipleSelection: YES];
    let _: () = msg_send![picker, setDelegate: delegate];
    picker
}

unsafe fn delegate() -> id {
    let existing = DELEGATE.get();
    if !existing.is_null() {
        return existing;
    }
    let mut decl = ClassDecl::new("AlaPickerDelegate", class!(NSObject)).unwrap();
    for name in [
        "PHPickerViewControllerDelegate",
        "UIImagePickerControllerDelegate",
        "UINavigationControllerDelegate",
        "UIDocumentPickerDelegate",
    ] {
        if let Some(protocol) = Protocol::get(name) {
            decl.add_protocol(protocol);
        }
    }
    decl.add_method(
        sel!(picker:didFinishPicking:),
        photos_done as extern "C" fn(&Object, Sel, id, id),
    );
    decl.add_method(
        sel!(imagePickerController:didFinishPickingMediaWithInfo:),
        camera_done as extern "C" fn(&Object, Sel, id, id),
    );
    decl.add_method(
        sel!(imagePickerControllerDidCancel:),
        camera_cancelled as extern "C" fn(&Object, Sel, id),
    );
    decl.add_method(
        sel!(documentPicker:didPickDocumentsAtURLs:),
        files_done as extern "C" fn(&Object, Sel, id, id),
    );
    decl.add_method(
        sel!(documentPickerWasCancelled:),
        files_cancelled as extern "C" fn(&Object, Sel, id),
    );
    let class = decl.register();
    let object: id = msg_send![class, new];
    DELEGATE.set(object);
    object
}

extern "C" fn photos_done(_: &Object, _: Sel, picker: id, results: id) {
    unsafe {
        let _: () = msg_send![picker, dismissViewControllerAnimated: YES completion: nil];
        let count: usize = msg_send![results, count];
        if count == 0 {
            finish(Vec::new());
            return;
        }
        let gathered = Arc::new(Mutex::new((vec![None; count], count)));
        for index in 0..count {
            let result: id = msg_send![results, objectAtIndex: index];
            let provider: id = msg_send![result, itemProvider];
            let types: id = msg_send![provider, registeredTypeIdentifiers];
            let first: id = msg_send![types, firstObject];
            let kind = nsstring_to_string(first).unwrap_or_else(|| "public.jpeg".to_owned());
            let suggested: id = msg_send![provider, suggestedName];
            let (mime, extension) = type_info(&kind);
            let name = file_name(nsstring_to_string(suggested), extension);
            let gathered = Arc::clone(&gathered);
            let completion = block2::RcBlock::new(move |data: *mut c_void, _error: *mut c_void| {
                let file = bytes(data.cast())
                    .map(|data| PickedFile::read(name.clone(), mime.clone(), data));
                let Ok(mut state) = gathered.lock() else {
                    return;
                };
                state.0[index] = file;
                state.1 -= 1;
                if state.1 == 0 {
                    let files: Vec<PickedFile> = state.0.drain(..).flatten().collect();
                    dispatch2::DispatchQueue::main().exec_async(move || finish(files));
                }
            });
            let _: id = msg_send![
                provider,
                loadDataRepresentationForTypeIdentifier: ns_string(&kind)
                completionHandler: &*completion as *const block2::Block<dyn Fn(*mut c_void, *mut c_void)> as id
            ];
        }
    }
}

extern "C" fn camera_done(_: &Object, _: Sel, picker: id, info: id) {
    unsafe {
        let _: () = msg_send![picker, dismissViewControllerAnimated: YES completion: nil];
        let image: id =
            msg_send![info, objectForKey: ns_string("UIImagePickerControllerOriginalImage")];
        let files = (!image.is_null())
            .then(|| bytes(UIImageJPEGRepresentation(image, JPEG_QUALITY)))
            .flatten()
            .map(|data| {
                PickedFile::read(format!("{PHOTO}.jpeg"), Some("image/jpeg".to_owned()), data)
            })
            .into_iter()
            .collect();
        finish(files);
    }
}

extern "C" fn camera_cancelled(_: &Object, _: Sel, picker: id) {
    unsafe {
        let _: () = msg_send![picker, dismissViewControllerAnimated: YES completion: nil];
    }
    finish(Vec::new());
}

extern "C" fn files_done(_: &Object, _: Sel, _: id, urls: id) {
    let limit = LIMIT.get();
    let mut files = Vec::new();
    unsafe {
        let count: usize = msg_send![urls, count];
        for index in 0..count {
            let url: id = msg_send![urls, objectAtIndex: index];
            let path: id = msg_send![url, path];
            if let Some(path) = nsstring_to_string(path) {
                files.extend(read_copy(Path::new(&path), limit));
            }
        }
    }
    finish(files);
}

extern "C" fn files_cancelled(_: &Object, _: Sel, _: id) {
    finish(Vec::new());
}

fn read_copy(path: &Path, limit: u64) -> Option<PickedFile> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let mime = path
        .extension()
        .and_then(|extension| unsafe { extension_mime(&extension.to_string_lossy()) });
    let size = std::fs::metadata(path).ok()?.len();
    let data = (size <= limit).then(|| std::fs::read(path).ok()).flatten();
    let _ = std::fs::remove_file(path);
    Some(PickedFile {
        name,
        mime,
        size,
        data,
    })
}

fn finish(files: Vec<PickedFile>) {
    if let Some(done) = PENDING.with_borrow_mut(Option::take) {
        done(files);
    }
}

fn file_name(suggested: Option<String>, extension: Option<String>) -> String {
    let stem = suggested
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| PHOTO.to_owned());
    match extension {
        Some(extension) if !stem.contains('.') => format!("{stem}.{extension}"),
        _ => stem,
    }
}

unsafe fn type_info(kind: &str) -> (Option<String>, Option<String>) {
    let utt: id = msg_send![class!(UTType), typeWithIdentifier: ns_string(kind)];
    if utt.is_null() {
        return (None, None);
    }
    let mime: id = msg_send![utt, preferredMIMEType];
    let extension: id = msg_send![utt, preferredFilenameExtension];
    (nsstring_to_string(mime), nsstring_to_string(extension))
}

unsafe fn extension_mime(extension: &str) -> Option<String> {
    let utt: id = msg_send![class!(UTType), typeWithFilenameExtension: ns_string(extension)];
    if utt.is_null() {
        return None;
    }
    let mime: id = msg_send![utt, preferredMIMEType];
    nsstring_to_string(mime)
}

unsafe fn bytes(data: id) -> Option<Vec<u8>> {
    if data.is_null() {
        return None;
    }
    let pointer: *const u8 = msg_send![data, bytes];
    let length: usize = msg_send![data, length];
    (!pointer.is_null() && length > 0).then(|| std::slice::from_raw_parts(pointer, length).to_vec())
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    fn UIImageJPEGRepresentation(image: id, quality: f64) -> id;
}

#[link(name = "PhotosUI", kind = "framework")]
unsafe extern "C" {}

#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}
