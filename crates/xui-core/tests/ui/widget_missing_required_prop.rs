use xui_macros::widget;

fn main() {}

#[widget]
struct Badge {
    label: String,
    #[default]
    tone: u8,
}

fn missing() -> BadgeProps {
    BadgeProps::builder().tone(2).build()
}
