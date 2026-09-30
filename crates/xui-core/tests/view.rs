//! Behavioural tests for `view!`, the SwiftUI-style spelling of the element DSL.
//!
//! `view!` shares `xui!`'s expansion, so these mostly pin that each `view!`
//! form lowers to the same element as its `xui!` counterpart, plus the control
//! flow that only `view!` has.

use xui_core::prelude::*;

use xui_core::widgets::WidgetType;
use xui_macros::{component, view, xui};

fn host(element: &ElementDesc) -> &WidgetDesc {
    match element {
        ElementDesc::Host(desc) => desc,
        other => panic!("expected a host element, got {other:?}"),
    }
}

fn texts(element: &ElementDesc) -> Vec<String> {
    host(element)
        .children
        .iter()
        .map(|child| {
            host(child)
                .widget
                .text()
                .expect("a text child")
                .as_str()
                .to_owned()
        })
        .collect()
}

#[test]
fn every_form_matches_its_xui_counterpart() {
    let pairs = [
        (view! { container() }, xui! { <container /> }),
        (
            view! { container(padding: EdgeInsets::all(8.0)) },
            xui! { <container padding={EdgeInsets::all(8.0)} /> },
        ),
        (
            view! { container().padding(EdgeInsets::all(8.0)) },
            xui! { <container padding={EdgeInsets::all(8.0)} /> },
        ),
        (
            view! { text("hello").color(Color::BLUE_500) },
            xui! { <text color={Color::BLUE_500}>{"hello"}</text> },
        ),
        (
            view! { row(gap: 4.0) { text("a") text("b") } },
            xui! { <row gap={4.0}><text>{"a"}</text><text>{"b"}</text></row> },
        ),
    ];
    for (view, xui) in pairs {
        assert_eq!(
            host(&view).widget.node_type(),
            host(&xui).widget.node_type()
        );
        assert_eq!(
            host(&view).widget.props_hash(),
            host(&xui).widget.props_hash()
        );
        assert_eq!(host(&view).children.len(), host(&xui).children.len());
    }
}

#[test]
fn a_multi_argument_modifier_is_passed_as_a_tuple() {
    let view = view! {
        container().box_shadow(Color::BLACK, Point::new(0.0, 2.0), 4.0, 0.0)
    };
    let xui = xui! {
        <container box_shadow={(Color::BLACK, Point::new(0.0, 2.0), 4.0, 0.0)} />
    };
    assert_eq!(
        host(&view).widget.props_hash(),
        host(&xui).widget.props_hash()
    );
}

#[test]
fn an_unnamed_argument_is_content() {
    // `text` reads it as its text...
    let label = view! { text("hello") };
    assert_eq!(host(&label).widget.text().unwrap().as_str(), "hello");

    // ...a container as children.
    let rows: Vec<ElementDesc> = vec![view! { container() }, view! { container() }];
    let list = view! { column(rows) };
    assert_eq!(host(&list).children.len(), 2);
}

#[test]
fn if_else_contributes_only_the_taken_branch() {
    for (flag, expected) in [(true, ["yes"]), (false, ["no"])] {
        let tree = view! {
            column {
                if flag {
                    text("yes")
                } else {
                    text("no")
                }
            }
        };
        assert_eq!(texts(&tree), expected);
    }
}

#[test]
fn else_if_and_if_let_chain() {
    let value: Option<u32> = Some(7);
    let tree = view! {
        column {
            if let Some(n) = value && n > 10 {
                text("big")
            } else if let Some(n) = value {
                text(format!("small {n}"))
            } else {
                text("none")
            }
        }
    };
    assert_eq!(texts(&tree), ["small 7"]);
}

#[test]
fn for_contributes_one_child_per_iteration() {
    let names = ["a", "b", "c"];
    let tree = view! {
        column {
            text("header")
            for name in names {
                text(name).key(name)
            }
            text("footer")
        }
    };
    assert_eq!(texts(&tree), ["header", "a", "b", "c", "footer"]);
    assert_eq!(
        host(&host(&tree).children[2]).widget.key(),
        Some("b".into())
    );
}

#[test]
fn match_arms_take_a_child_or_a_child_list() {
    enum Mode {
        One,
        Two,
        Other(&'static str),
    }
    let render = |mode: Mode| {
        view! {
            column {
                match mode {
                    Mode::One => text("one"),
                    Mode::Two => {
                        text("two")
                        text("two")
                    }
                    Mode::Other(label) if label.is_empty() => {}
                    Mode::Other(label) => text(label),
                }
            }
        }
    };
    assert_eq!(texts(&render(Mode::One)), ["one"]);
    assert_eq!(texts(&render(Mode::Two)), ["two", "two"]);
    assert_eq!(texts(&render(Mode::Other(""))), Vec::<String>::new());
    assert_eq!(texts(&render(Mode::Other("x"))), ["x"]);
}

#[test]
fn let_bindings_are_visible_to_later_siblings() {
    let tree = view! {
        column {
            let greeting = format!("hi {}", 3);
            text(greeting.clone())
            text(greeting)
        }
    };
    assert_eq!(texts(&tree), ["hi 3", "hi 3"]);
}

#[test]
fn dot_dot_expressions_are_spliced() {
    let extra: Vec<ElementDesc> = vec![view! { text("b") }, view! { text("c") }];
    let single = view! { text("d") };
    let last = view! { text("f") };
    let tree = view! {
        column {
            text("a")
            ..extra
            ..single
            ..["e"].map(|label| view! { text(label) })
            ..last
        }
    };
    assert_eq!(texts(&tree), ["a", "b", "c", "d", "e", "f"]);
}

#[test]
fn a_match_arm_can_be_a_single_splice() {
    let rows = vec![view! { text("x") }];
    let tree = view! {
        column {
            match rows.is_empty() {
                true => text("empty"),
                false => ..rows,
            }
        }
    };
    assert_eq!(texts(&tree), ["x"]);
}

#[component]
fn greeting(name: &String) {
    view! { text(name.clone()) }
}

#[component]
#[defaults(children = Vec::new())]
fn shell(children: &Vec<ElementDesc>) {
    view! { container(children.to_vec()) }
}

#[test]
fn components_take_named_props_and_bodies() {
    let element = view! { greeting(name: String::from("world")).key("hi") };
    match element {
        ElementDesc::Component(desc) => assert_eq!(desc.key, Some("hi".into())),
        other => panic!("expected a component element, got {other:?}"),
    }

    let element = view! {
        shell {
            container()
            container()
        }
    };
    assert!(matches!(element, ElementDesc::Component(_)));
}

#[test]
fn a_path_can_be_used_as_an_element() {
    let element = view! { xui_core::widgets::container() };
    assert_eq!(host(&element).widget.node_type(), WidgetType::Container);
}
