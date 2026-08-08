use std::fmt as formatting;
pub trait Render { fn render(&self) -> String; }
pub struct View;
impl Render for View {
    /// Render a view.
    fn render(&self) -> String {
        let value = helper();
        if value.is_empty() { return formatting::format(format_args!("ok")); }
        value
    }
}
#[test]
fn renders() { assert_eq!(View.render(), "ok"); }
