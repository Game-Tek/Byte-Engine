pub trait Model: for<'de> serde::Deserialize<'de> {
    fn get_class() -> &'static str;
}