use crate::ios::{CGRect, id, nil, ns_array, ns_string, nsstring_to_string};
use crate::pick::PickSource;
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{BOOL, NO, Object, Sel, YES},
    sel, sel_impl,
};
use std::{
    cell::{Cell, RefCell},
    ptr,
};

const VIDEO: &str = "vide";
const QR: &str = "org.iso.QRCode";
const FILL: &str = "AVLayerVideoGravityResizeAspectFill";
const FULL_SCREEN: isize = 0;
const CLOSE_BUTTON: isize = 7;
const TOUCH_UP_INSIDE: usize = 1 << 6;
const CLOSE_INSET: f64 = 16.0;

type Done = Box<dyn FnOnce(Option<String>)>;

thread_local! {
    static PENDING: RefCell<Option<Done>> = const { RefCell::new(None) };
    static SESSION: Cell<id> = const { Cell::new(ptr::null_mut()) };
    static SCREEN: Cell<id> = const { Cell::new(ptr::null_mut()) };
    static DELEGATE: Cell<id> = const { Cell::new(ptr::null_mut()) };
}

pub fn can_scan() -> bool {
    unsafe { camera().is_some() || super::picker::can_pick(PickSource::Photos) }
}

pub fn scan_code(done: impl FnOnce(Option<String>) + 'static) {
    if let Some(previous) = PENDING.with_borrow_mut(|pending| pending.replace(Box::new(done))) {
        previous(None);
    }
    match unsafe { camera() } {
        Some(device) => unsafe { watch(device) },
        None => super::picker::pick(PickSource::Photos, u64::MAX, |files| {
            let code = files
                .iter()
                .filter_map(|file| file.data.as_deref())
                .find_map(|data| unsafe { read_code(data) });
            finish(code);
        }),
    }
}

unsafe fn camera() -> Option<id> {
    let device: id =
        msg_send![class!(AVCaptureDevice), defaultDeviceWithMediaType: ns_string(VIDEO)];
    (!device.is_null()).then_some(device)
}

unsafe fn watch(device: id) {
    let Some(host) = super::picker::top_controller() else {
        finish(None);
        return;
    };
    let mut error: id = nil;
    let input: id =
        msg_send![class!(AVCaptureDeviceInput), deviceInputWithDevice: device error: &mut error];
    if input.is_null() {
        finish(None);
        return;
    }
    let session: id = msg_send![class!(AVCaptureSession), new];
    let added: BOOL = msg_send![session, canAddInput: input];
    let output: id = msg_send![class!(AVCaptureMetadataOutput), new];
    let fits: BOOL = msg_send![session, canAddOutput: output];
    if added == NO || fits == NO {
        let _: () = msg_send![output, release];
        let _: () = msg_send![session, release];
        finish(None);
        return;
    }
    let _: () = msg_send![session, addInput: input];
    let _: () = msg_send![session, addOutput: output];
    let delegate = delegate();
    let main = dispatch2::DispatchQueue::main() as *const dispatch2::DispatchQueue as id;
    let _: () = msg_send![output, setMetadataObjectsDelegate: delegate queue: main];
    let _: () = msg_send![output, setMetadataObjectTypes: ns_array(&[ns_string(QR)])];
    let _: () = msg_send![output, release];

    let screen: id = msg_send![class!(UIViewController), new];
    let view: id = msg_send![screen, view];
    let black: id = msg_send![class!(UIColor), blackColor];
    let _: () = msg_send![view, setBackgroundColor: black];
    let preview: id = msg_send![class!(AVCaptureVideoPreviewLayer), layerWithSession: session];
    let _: () = msg_send![preview, setVideoGravity: ns_string(FILL)];
    let host_view: id = msg_send![host, view];
    let window: id = msg_send![host_view, window];
    let bounds: CGRect = msg_send![window, bounds];
    let _: () = msg_send![preview, setFrame: bounds];
    let layer: id = msg_send![view, layer];
    let _: () = msg_send![layer, addSublayer: preview];

    let close: id = msg_send![class!(UIButton), buttonWithType: CLOSE_BUTTON];
    let _: () = msg_send![close, addTarget: delegate action: sel!(close:) forControlEvents: TOUCH_UP_INSIDE];
    let _: () = msg_send![close, setTranslatesAutoresizingMaskIntoConstraints: NO];
    let _: () = msg_send![view, addSubview: close];
    let guide: id = msg_send![view, safeAreaLayoutGuide];
    let close_top: id = msg_send![close, topAnchor];
    let guide_top: id = msg_send![guide, topAnchor];
    let close_end: id = msg_send![close, trailingAnchor];
    let guide_end: id = msg_send![guide, trailingAnchor];
    let top: id = msg_send![close_top, constraintEqualToAnchor: guide_top constant: CLOSE_INSET];
    let trailing: id =
        msg_send![close_end, constraintEqualToAnchor: guide_end constant: -CLOSE_INSET];
    let _: () =
        msg_send![class!(NSLayoutConstraint), activateConstraints: ns_array(&[top, trailing])];

    let _: () = msg_send![screen, setModalPresentationStyle: FULL_SCREEN];
    let _: () = msg_send![host, presentViewController: screen animated: YES completion: nil];
    SESSION.set(session);
    SCREEN.set(screen);
    let running = session as usize;
    let _: () = msg_send![session, retain];
    std::thread::spawn(move || {
        let session = running as id;
        let _: () = msg_send![session, startRunning];
        let _: () = msg_send![session, release];
    });
}

unsafe fn delegate() -> id {
    let existing = DELEGATE.get();
    if !existing.is_null() {
        return existing;
    }
    let mut decl = ClassDecl::new("AlaCodeScanner", class!(NSObject)).unwrap();
    if let Some(protocol) = objc::runtime::Protocol::get("AVCaptureMetadataOutputObjectsDelegate") {
        decl.add_protocol(protocol);
    }
    decl.add_method(
        sel!(captureOutput:didOutputMetadataObjects:fromConnection:),
        seen as extern "C" fn(&Object, Sel, id, id, id),
    );
    decl.add_method(sel!(close:), closed as extern "C" fn(&Object, Sel, id));
    let class = decl.register();
    let object: id = msg_send![class, new];
    DELEGATE.set(object);
    object
}

extern "C" fn seen(_: &Object, _: Sel, _: id, objects: id, _: id) {
    let code = unsafe {
        let count: usize = msg_send![objects, count];
        (0..count).find_map(|index| {
            let object: id = msg_send![objects, objectAtIndex: index];
            let value: id = msg_send![object, stringValue];
            nsstring_to_string(value)
        })
    };
    if code.is_some() {
        unsafe { stop() };
        finish(code);
    }
}

extern "C" fn closed(_: &Object, _: Sel, _: id) {
    unsafe { stop() };
    finish(None);
}

unsafe fn stop() {
    let session = SESSION.replace(ptr::null_mut());
    if !session.is_null() {
        let _: () = msg_send![session, stopRunning];
        let _: () = msg_send![session, release];
    }
    let screen = SCREEN.replace(ptr::null_mut());
    if !screen.is_null() {
        let _: () = msg_send![screen, dismissViewControllerAnimated: YES completion: nil];
        let _: () = msg_send![screen, release];
    }
}

unsafe fn read_code(data: &[u8]) -> Option<String> {
    let bytes: id = msg_send![class!(NSData), dataWithBytes: data.as_ptr() length: data.len()];
    let image: id = msg_send![class!(CIImage), imageWithData: bytes];
    if image.is_null() {
        return None;
    }
    let options: id = msg_send![
        class!(NSDictionary),
        dictionaryWithObject: ns_string("CIDetectorAccuracyHigh")
        forKey: ns_string("CIDetectorAccuracy")
    ];
    let detector: id = msg_send![
        class!(CIDetector),
        detectorOfType: ns_string("CIDetectorTypeQRCode")
        context: nil
        options: options
    ];
    if detector.is_null() {
        return None;
    }
    let features: id = msg_send![detector, featuresInImage: image];
    let count: usize = msg_send![features, count];
    (0..count).find_map(|index| {
        let feature: id = msg_send![features, objectAtIndex: index];
        let message: id = msg_send![feature, messageString];
        nsstring_to_string(message)
    })
}

fn finish(code: Option<String>) {
    if let Some(done) = PENDING.with_borrow_mut(Option::take) {
        done(code);
    }
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreImage", kind = "framework")]
unsafe extern "C" {}
