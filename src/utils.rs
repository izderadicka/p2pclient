use std::borrow::Cow;

use crate::error::*;

pub fn next_item<'a, I: Iterator<Item = &'a str>>(items: &mut I, what: &str) -> Result<&'a str> {
    if let Some(item) = items.next() {
        Ok(item)
    } else {
        Err(Error::msg(format!("Expected {} in input line", what)))
    }
}

pub fn rest_of<'a, I: Iterator<Item = &'a str>>(items: I) -> Result<String> {
    let txt: Vec<_> = items.collect();
    let txt = txt.join(" ");
    if txt.is_empty() {
        Err(Error::msg(format!("Empty value")))
    } else {
        Ok(txt)
    }
}

pub trait Printable {
    fn printable(&self) -> Cow<str>;
}

pub trait PrintableList<'a> {
    fn printable_list(&'a self) -> String;
}

impl<T> Printable for T
where
    T: AsRef<[u8]>,
{
    fn printable(&self) -> Cow<str> {
        match std::str::from_utf8(self.as_ref()) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => Cow::Owned(bs58::encode(self).into_string()),
        }
    }
}

impl<'a, T> PrintableList<'a> for T
where
    &'a T: IntoIterator,
    T: 'a,
    <&'a T as IntoIterator>::Item: std::string::ToString,
{
    fn printable_list(&'a self) -> String {
        self.into_iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
