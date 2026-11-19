#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickSource {
    Photos,
    Camera,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickedFile {
    pub name: String,
    pub mime: Option<String>,
    pub size: u64,
    pub data: Option<Vec<u8>>,
}

impl PickedFile {
    pub(crate) fn read(name: String, mime: Option<String>, data: Vec<u8>) -> Self {
        Self {
            name,
            mime,
            size: data.len() as u64,
            data: Some(data),
        }
    }
}
