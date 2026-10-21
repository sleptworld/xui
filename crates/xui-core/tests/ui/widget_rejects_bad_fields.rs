use xui_macros::widget;

fn main() {}

#[widget]
#[defaults(count = 1)]
struct DefaultedState {
    #[state]
    count: u32,
}

#[widget]
#[derive(Clone)]
struct BareDerive {
    value: u32,
}

#[widget]
struct StateInitializer {
    #[state(1)]
    count: u32,
}

#[widget(update = sync)]
struct WithArguments {
    value: u32,
}
