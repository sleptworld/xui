use xui_macros::component;

fn main() {}

struct Opaque;

#[component(memo)]
fn row(label: &String, opaque: &Opaque) {
    let _ = (label, opaque);
    unimplemented!()
}
