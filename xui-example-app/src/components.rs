use xui_core::prelude::*;
use xui_macros::{component, xui};

#[component]
#[defaults(label = "Label".to_string(), name = "Name".to_string())]
pub fn test_badge(label: &String, name: &String) {
    xui! {
        <column>
            <text> {label} </text>
            <text> {name} </text>
        </column>
    }
}
