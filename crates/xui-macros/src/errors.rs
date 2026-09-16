//! Error accumulation.
//!
//! Bailing on the first problem makes a large `xui!` block a one-error-per-build
//! grind. Where a pass can keep going, it collects into [`Errors`] and reports
//! everything at once.

use syn::Error;

#[derive(Default)]
pub struct Errors(Option<Error>);

impl Errors {
    pub fn push(&mut self, error: Error) {
        match &mut self.0 {
            Some(existing) => existing.combine(error),
            None => self.0 = Some(error),
        }
    }

    pub fn into_result(self) -> syn::Result<()> {
        match self.0 {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
