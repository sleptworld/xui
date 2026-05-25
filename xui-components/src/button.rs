use xui::{EdgeInsets, Size, component_fn};

#[derive(Debug, Hash)]
pub struct ButtonProps {
    pub text: String,
}

component_fn! {

    pub fn pbutton(ButtonProps{text}: &ButtonProps) {
        <container padding={EdgeInsets::all(8.0)} size={Some(Size::new(20.0,10.0))}>
            <text> {text} </text>
        </container>
    }

}
