//! Writing text to the system clipboard.

pub fn set(text: &str) {
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
        Ok(()) => {}
        Err(e) => eprintln!("Burrow: clipboard: {e}"),
    }
}
