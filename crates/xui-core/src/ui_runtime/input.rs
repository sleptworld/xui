//! Writes the event system makes back into the runtime while dispatching.

use crate::event_system;
use crate::event_system::EventState;
use crate::event_system::translator::EventTranslator;
use crate::focus::FocusManager;
use crate::text::TextHost;
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::state::HostWorkFlags;
use xui_interface::events::RawEvent;
use xui_interface::{EventResult, NodeId, TextBackend, WidgetState, WidgetUpdateFlags};

impl UiRuntime {
    pub(crate) fn focus_manager_mut(&mut self) -> &mut FocusManager {
        &mut self.interaction_system.focus
    }

    pub(crate) fn event_state_mut(&mut self) -> &mut EventState {
        &mut self.interaction_system.event_state
    }

    /// Schedules the work an event handler asked for on `id`.
    ///
    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn request_update(&mut self, id: NodeId, flags: WidgetUpdateFlags) {
        self.mark_work(id, HostWorkFlags::from_widget_update(flags));
    }

    /// # Preconditions
    /// - `id` is live.
    pub(crate) fn set_widget_state_flag(&mut self, id: NodeId, flag: WidgetState, enabled: bool) {
        let host = &mut self.hosts[id];
        let before = host.state;
        host.state.set(flag, enabled);
        if before != host.state {
            host.state_before_change.get_or_insert(before);
            self.mark_work(id, HostWorkFlags::SYNC_STATE_CHANGE);
        }
    }

    #[inline(always)]
    pub(crate) fn dispatch_event<T: TextBackend>(
        &mut self,
        text: &TextHost<T>,
        translator: &mut EventTranslator,
        event: RawEvent,
    ) -> EventResult {
        event_system::dispatch_event(self, text, translator, event)
    }
}
