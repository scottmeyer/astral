use super::*;
pub(crate) struct Directory;
pub(crate) struct Guard;
fn no<T>() -> Result<T> {
    Err(error(
        "HOOK_UNSUPPORTED_PLATFORM",
        "hook storage requires Unix",
    ))
}
impl Directory {
    pub fn open(_: &Path, _: bool) -> Result<Self> {
        no()
    }
    pub fn identity(&self) -> Result<String> {
        no()
    }
    pub fn child(&self, _: &str, _: bool, _: bool) -> Result<Option<Self>> {
        no()
    }
    pub fn read(&self, _: &str, _: usize, _: bool) -> Result<Option<Snapshot>> {
        no()
    }
    pub fn write(&self, _: &str, _: &[u8], _: Option<&str>, _: u32) -> Result<()> {
        no()
    }
    pub fn lock(&self) -> Result<Guard> {
        no()
    }
}
