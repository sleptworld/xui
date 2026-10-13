//! Observe a widget's size after layout, without changing its layout rules.
//!
//! ```
//! use xui_core::prelude::*;
//!
//! fn panel(cx: &mut HookContext<'_>) -> ElementDesc {
//!     let size = cx.use_size();
//!     let latest = size.get(); // None until the widget has been laid out
//!     container().observe_size(size).into_element_desc(Vec::new())
//! }
//! ```

use std::cell::Cell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use xui_interface::{NodeId, Size};

use crate::fiber::FiberId;
use crate::lanes::DEFAULT_LANE;
use crate::state::Scheduler;

struct SizeState {
    node: Cell<Option<NodeId>>,
    size: Cell<Option<Size<f32>>>,
    owner: FiberId,
    scheduler: Scheduler,
}

/// A stable subscription created by [`crate::state::HookContext::use_size`].
///
/// Bind it to one widget using `.observe_size(observer.clone())`. Sizes are
/// the final layout border box in logical pixels, including padding, excluding
/// transforms and overflowing content. A change schedules the owning component
/// on the ordinary update lane, for a subsequent render. There is no layout
/// containment and no layout/rebuild loop before painting.
#[derive(Clone)]
pub struct SizeObserver {
    inner: Rc<SizeState>,
}

impl SizeObserver {
    pub(crate) fn new(owner: FiberId, scheduler: Scheduler) -> Self {
        Self {
            inner: Rc::new(SizeState {
                node: Cell::new(None),
                size: Cell::new(None),
                owner,
                scheduler,
            }),
        }
    }

    /// The latest completed layout size, or `None` before layout or after
    /// detaching the observer. Reading does not trigger layout.
    pub fn get(&self) -> Option<Size<f32>> {
        self.inner.size.get()
    }

    pub(crate) fn bind(&self, node: NodeId) {
        assert!(
            self.inner.node.get().is_none_or(|bound| bound == node),
            "a use_size handle can observe only one widget at a time"
        );
        self.inner.node.set(Some(node));
    }

    pub(crate) fn unbind(&self, node: NodeId) -> bool {
        if self.inner.node.get() == Some(node) {
            self.inner.node.set(None);
            return self.inner.size.take().is_some();
        }
        false
    }

    pub(crate) fn offer(&self, node: NodeId, size: Size<f32>) {
        if self.inner.node.get() == Some(node) {
            self.publish(Some(size));
        }
    }

    fn publish(&self, size: Option<Size<f32>>) {
        if self.inner.size.replace(size) != size {
            self.notify();
        }
    }

    pub(crate) fn notify(&self) {
        self.inner
            .scheduler
            .mark_component_dirty(self.inner.owner, DEFAULT_LANE);
    }
}

impl fmt::Debug for SizeObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SizeObserver")
            .field("node", &self.inner.node.get())
            .field("size", &self.get())
            .finish()
    }
}

impl PartialEq for SizeObserver {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for SizeObserver {}

impl Hash for SizeObserver {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.inner).hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::app::{App, ComponentFn};
    use crate::element::ElementDesc;
    use crate::render::MockRenderBackend;
    use crate::runtime::{GuiRuntime, RuntimeEvent};
    use crate::state::{HookContext, State};
    use crate::text::testing::ZeroTextBackend;
    use crate::widgets::{container, text};
    use xui_interface::{EdgeInsets, Style};

    thread_local! {
        static LOG: RefCell<Vec<Option<Size<f32>>>> = const { RefCell::new(Vec::new()) };
        static OBSERVER: RefCell<Option<SizeObserver>> = const { RefCell::new(None) };
        static VISIBLE: RefCell<Option<State<bool>>> = const { RefCell::new(None) };
    }

    fn observe(cx: &mut HookContext<'_>) -> SizeObserver {
        let observer = cx.use_size();
        LOG.with(|log| log.borrow_mut().push(observer.get()));
        OBSERVER.with(|slot| *slot.borrow_mut() = Some(observer.clone()));
        observer
    }

    fn take_log() -> Vec<Option<Size<f32>>> {
        LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
    }

    fn latest() -> Option<Size<f32>> {
        OBSERVER.with(|slot| slot.borrow().as_ref().unwrap().get())
    }

    fn runtime(root: ComponentFn) -> GuiRuntime<MockRenderBackend, ZeroTextBackend> {
        take_log();
        let mut runtime = GuiRuntime::new(
            App::new(root),
            MockRenderBackend::default(),
            ZeroTextBackend,
        );
        runtime.handle_event(RuntimeEvent::Resize(Size::new(400.0, 300.0)));
        runtime
    }

    fn root(cx: &mut HookContext<'_>) -> ElementDesc {
        let observer = observe(cx);
        container()
            .observe_size(observer)
            .style(Style::new().padding(EdgeInsets::all(10.0)))
            .into_element_desc(vec![
                container()
                    .style(Style::new().height(40.0))
                    .into_element_desc(Vec::new()),
            ])
    }

    #[test]
    fn size_is_published_after_layout_and_renders_later_without_containment() {
        let mut runtime = runtime(root);
        runtime.frame().unwrap();
        assert_eq!(take_log(), [None], "no rebuild inside the first frame");
        assert_eq!(
            latest(),
            Some(Size::new(400.0, 60.0)),
            "padding and content size the box normally"
        );
        assert_eq!(runtime.app().ui_runtime().stats.layout_passes, 1);
        assert!(runtime.app().is_dirty());

        runtime.frame().unwrap();
        assert_eq!(take_log(), [Some(Size::new(400.0, 60.0))]);
        assert!(
            !runtime.app().is_dirty(),
            "recommitting the same observer preserves its size"
        );
        let passes = runtime.app().ui_runtime().stats.layout_passes;
        assert!(!runtime.frame().unwrap().rendered);
        assert!(take_log().is_empty());
        assert_eq!(runtime.app().ui_runtime().stats.layout_passes, passes);

        runtime.handle_event(RuntimeEvent::Resize(Size::new(250.0, 300.0)));
        runtime.frame().unwrap();
        assert!(
            take_log().is_empty(),
            "resize publishes without invoking the component"
        );
        assert_eq!(latest(), Some(Size::new(250.0, 60.0)));
        runtime.frame().unwrap();
        assert_eq!(take_log(), [Some(Size::new(250.0, 60.0))]);
        assert!(!runtime.app().is_dirty());
    }

    fn toggle_root(cx: &mut HookContext<'_>) -> ElementDesc {
        let observer = observe(cx);
        let visible = cx.use_state(|| true);
        VISIBLE.with(|slot| *slot.borrow_mut() = Some(visible));
        let child = container().style(Style::new().width(120.0).height(40.0));
        let child = if *visible.get() {
            child.observe_size(observer)
        } else {
            child
        };
        container().into_element_desc(vec![child.into_element_desc(Vec::new())])
    }

    fn set_visible(value: bool) {
        slot::Runtime::with_phase(slot::RenderPhase::Event, || {
            VISIBLE.with(|slot| slot.borrow().unwrap().set(value));
        });
    }

    #[test]
    fn attaching_and_detaching_on_an_existing_node_notifies_after_commit() {
        let mut runtime = runtime(toggle_root);
        runtime.frame().unwrap();
        runtime.frame().unwrap();
        take_log();
        let passes = runtime.app().ui_runtime().stats.layout_passes;

        set_visible(false);
        runtime.frame().unwrap();
        assert_eq!(latest(), None);
        assert!(
            runtime.app().is_dirty(),
            "detachment update survives the committing render"
        );
        runtime.frame().unwrap();
        assert_eq!(take_log(), [Some(Size::new(120.0, 40.0)), None]);
        assert!(!runtime.app().is_dirty());

        set_visible(true);
        runtime.frame().unwrap();
        assert_eq!(latest(), Some(Size::new(120.0, 40.0)));
        runtime.frame().unwrap();
        assert_eq!(take_log(), [None, Some(Size::new(120.0, 40.0))]);
        assert!(!runtime.app().is_dirty());
        assert_eq!(
            runtime.app().ui_runtime().stats.layout_passes,
            passes,
            "subscription changes do not invalidate geometry"
        );
    }

    fn leaf_root(cx: &mut HookContext<'_>) -> ElementDesc {
        text()
            .text("observed leaf")
            .observe_size(observe(cx))
            .style(Style::new().width(80.0).height(20.0))
            .into_element_desc()
    }

    #[test]
    fn observes_a_leaf_and_clears_the_handle_when_the_runtime_is_dropped() {
        let mut runtime = runtime(leaf_root);
        runtime.frame().unwrap();
        assert_eq!(latest(), Some(Size::new(80.0, 20.0)));
        runtime.frame().unwrap();
        assert_eq!(take_log(), [None, Some(Size::new(80.0, 20.0))]);
        drop(runtime);
        assert_eq!(latest(), None);
    }

    fn removable_root(cx: &mut HookContext<'_>) -> ElementDesc {
        let observer = observe(cx);
        let visible = cx.use_state(|| true);
        VISIBLE.with(|slot| *slot.borrow_mut() = Some(visible));
        let children = if *visible.get() {
            vec![
                container()
                    .observe_size(observer)
                    .style(Style::new().width(90.0).height(30.0))
                    .into_element_desc(Vec::new()),
            ]
        } else {
            Vec::new()
        };
        container().into_element_desc(children)
    }

    #[test]
    fn removal_clears_the_subscription_and_the_handle_can_observe_a_new_node() {
        let mut runtime = runtime(removable_root);
        runtime.frame().unwrap();
        runtime.frame().unwrap();
        take_log();
        set_visible(false);
        runtime.frame().unwrap();
        assert_eq!(latest(), None);
        runtime.frame().unwrap();
        assert_eq!(take_log(), [Some(Size::new(90.0, 30.0)), None]);
        assert!(!runtime.app().is_dirty());
        set_visible(true);
        runtime.frame().unwrap();
        assert_eq!(latest(), Some(Size::new(90.0, 30.0)));
        runtime.frame().unwrap();
        assert_eq!(take_log(), [None, Some(Size::new(90.0, 30.0))]);
        assert!(!runtime.app().is_dirty());
    }
}
