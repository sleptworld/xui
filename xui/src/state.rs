use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};

use slot::unsync::Slot;
use slot::{Pointer, RenderPhase as SlotRenderPhase, Runtime as SlotRuntime, Scope};
use tokio::runtime::Handle as TokioHandle;
use tokio::task::JoinHandle;

use crate::fiber::FiberId;
use crate::lanes::{
    DEFAULT_LANE, Lane, LaneRoot, Lanes, NO_LANES, RETRY_LANE, current_update_lane,
    includes_some_lane,
};

type HookKey = (FiberId, usize);
type HookApply = Box<dyn FnOnce(&dyn Any)>;
type AsyncHookApply = Box<dyn FnOnce(&dyn Any) + Send>;
type HookSlot = ();
type WakeCallback = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Clone, Copy)]
struct AsyncScope {
    owner: FiberId,
    hook_index: usize,
    generation: u64,
}

pub(crate) struct AsyncMessage {
    owner: FiberId,
    hook_index: usize,
    lane: Lane,
    scope: Option<AsyncScope>,
    apply: AsyncHookApply,
}

#[derive(Clone)]
pub struct AsyncDispatcher {
    sender: mpsc::Sender<AsyncMessage>,
    wake: Arc<Mutex<Option<WakeCallback>>>,
}

impl AsyncDispatcher {
    pub(crate) fn new(sender: mpsc::Sender<AsyncMessage>) -> Self {
        Self {
            sender,
            wake: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn noop() -> Self {
        let (sender, _) = mpsc::channel();
        Self::new(sender)
    }

    pub(crate) fn set_wake_callback(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.wake.lock().expect("async wake callback poisoned") = Some(Arc::new(wake));
    }

    fn dispatch_hook_update<T: 'static>(
        &self,
        owner: FiberId,
        hook_index: usize,
        lane: Lane,
        scope: Option<AsyncScope>,
        update: impl FnOnce(&mut T) + Send + 'static,
    ) {
        let message = AsyncMessage {
            owner,
            hook_index,
            lane,
            scope,
            apply: Box::new(move |value| {
                let state = value
                    .downcast_ref::<Pointer<Slot, T>>()
                    .expect("async hook update type changed");
                write_slot(*state, update);
            }),
        };

        if self.sender.send(message).is_ok() {
            if let Some(wake) = self
                .wake
                .lock()
                .expect("async wake callback poisoned")
                .as_ref()
                .cloned()
            {
                wake();
            }
        }
    }
}

#[derive(Default)]
pub struct HookStorage {
    scope: Scope,
    slots: Vec<Pointer<Slot, HookSlot>>,
    cursor: usize,
}

struct StateSlot<T> {
    value: T,
    owner: FiberId,
    hook_index: usize,
    scheduler: Scheduler,
    dispatcher: AsyncDispatcher,
}

pub struct State<T: 'static> {
    inner: Pointer<Slot, StateSlot<T>>,
}

impl<T: 'static> Copy for State<T> {}

impl<T: 'static> Clone for State<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> fmt::Debug for State<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State").field("inner", &self.inner).finish()
    }
}

impl<T: 'static> PartialEq for State<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T: 'static> Eq for State<T> {}

impl<T: 'static> Hash for State<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl<T: 'static> State<T> {
    pub fn set(&self, value: T) {
        self.update(move |slot| {
            *slot = value;
        });
    }

    pub fn update(&self, update: impl FnOnce(&mut T) + 'static) {
        let state = read_slot(self.inner);
        state
            .scheduler
            .enqueue_hook_update(state.owner, state.hook_index, update);
    }
}

impl<T: Send + 'static> State<T> {
    pub fn setter(&self) -> StateSetter<T> {
        let state = read_slot(self.inner);
        StateSetter {
            dispatcher: state.dispatcher.clone(),
            owner: state.owner,
            hook_index: state.hook_index,
            scope: None,
            _marker: PhantomData,
        }
    }
}

impl<T: Clone + 'static> State<T> {
    pub fn get(&self) -> &T {
        &read_slot(self.inner).value
    }
}

pub struct StateSetter<T: Send + 'static> {
    dispatcher: AsyncDispatcher,
    owner: FiberId,
    hook_index: usize,
    scope: Option<AsyncScope>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Send + 'static> Clone for StateSetter<T> {
    fn clone(&self) -> Self {
        Self {
            dispatcher: self.dispatcher.clone(),
            owner: self.owner,
            hook_index: self.hook_index,
            scope: self.scope,
            _marker: PhantomData,
        }
    }
}

impl<T: Send + 'static> StateSetter<T> {
    fn with_scope(mut self, scope: AsyncScope) -> Self {
        self.scope = Some(scope);
        self
    }

    pub fn set(&self, value: T) {
        self.update(move |slot| {
            *slot = value;
        });
    }

    pub fn update(&self, update: impl FnOnce(&mut T) + Send + 'static) {
        self.dispatcher.dispatch_hook_update::<StateSlot<T>>(
            self.owner,
            self.hook_index,
            DEFAULT_LANE,
            self.scope,
            move |state| update(&mut state.value),
        );
    }
}

fn read_slot<T: 'static>(pointer: Pointer<Slot, T>) -> &'static T {
    SlotRuntime::with_phase(SlotRenderPhase::Render, || unsafe {
        pointer.try_read().unwrap()
    })
}

fn write_slot<T: 'static>(pointer: Pointer<Slot, T>, value: impl for<'a> FnOnce(&'a mut T)) {
    SlotRuntime::with_phase(SlotRenderPhase::Effect, || unsafe {
        let p = pointer.try_write().unwrap();
        value(p);
    });
}

impl HookStorage {
    pub fn begin(&mut self) {
        self.cursor = 0;
    }

    fn next_slot<T: 'static>(
        &mut self,
        init: impl FnOnce(usize) -> T,
    ) -> (usize, Pointer<Slot, T>) {
        let index = self.cursor;
        self.cursor += 1;

        if index == self.slots.len() {
            let pointer = self.scope.insert(init(index));
            self.slots.push(unsafe { pointer.cast::<HookSlot>() });
            return (index, pointer);
        }

        (index, unsafe { self.slots[index].cast::<T>() })
    }
}

struct CallbackState<D, T> {
    deps: D,
    callback: Rc<RefCell<T>>,
}

#[derive(Clone)]
pub struct Callback<T> {
    callback: Rc<RefCell<T>>,
}

impl<T> Callback<T> {
    pub fn call_mut<R>(&self, call: impl FnOnce(&mut T) -> R) -> R {
        let mut callback = self.callback.borrow_mut();
        call(&mut callback)
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.callback, &other.callback)
    }
}

pub struct HookContext<'a> {
    storage: &'a mut HookStorage,
    owner: FiberId,
    scheduler: Scheduler,
    render_lanes: Lanes,
    dispatcher: AsyncDispatcher,
    tokio_handle: Option<TokioHandle>,
}

impl<'a> HookContext<'a> {
    pub fn new(
        storage: &'a mut HookStorage,
        owner: FiberId,
        scheduler: Scheduler,
        render_lanes: Lanes,
    ) -> Self {
        storage.begin();
        Self::new_with_async(
            storage,
            owner,
            scheduler,
            render_lanes,
            AsyncDispatcher::noop(),
            None,
        )
    }

    pub(crate) fn new_with_async(
        storage: &'a mut HookStorage,
        owner: FiberId,
        scheduler: Scheduler,
        render_lanes: Lanes,
        dispatcher: AsyncDispatcher,
        tokio_handle: Option<TokioHandle>,
    ) -> Self {
        storage.begin();
        Self {
            storage,
            owner,
            scheduler,
            render_lanes,
            dispatcher,
            tokio_handle,
        }
    }

    pub fn use_state<T: Clone + 'static>(&mut self, init: impl FnOnce() -> T) -> State<T> {
        let scheduler = self.scheduler.clone();
        let dispatcher = self.dispatcher.clone();
        let owner = self.owner;
        let (hook_index, state) = self.storage.next_slot(|hook_index| StateSlot {
            value: init(),
            owner,
            hook_index,
            scheduler,
            dispatcher,
        });

        self.scheduler
            .apply_hook_updates(self.owner, hook_index, self.render_lanes, &state);
        State { inner: state }
    }

    pub fn use_callback<D, T>(&mut self, deps: D, init: impl FnOnce() -> T) -> Callback<T>
    where
        D: PartialEq + 'static,
        T: 'static,
    {
        let mut next_deps = Some(deps);
        let mut next_init = Some(init);
        let (_, state) = self.storage.next_slot(|_| {
            let deps = next_deps
                .take()
                .expect("callback deps should be available for new hook slot");
            let init = next_init
                .take()
                .expect("callback init should be available for new hook slot");
            RefCell::new(CallbackState {
                deps,
                callback: Rc::new(RefCell::new(init())),
            })
        });
        let state = read_slot(state);

        if let Some(deps) = next_deps.take() {
            let mut state = state.borrow_mut();
            if state.deps != deps {
                state.deps = deps;
                let init = next_init
                    .take()
                    .expect("callback init should be available when deps change");
                *state.callback.borrow_mut() = init();
            }
        }

        Callback {
            callback: state.borrow().callback.clone(),
        }
    }

    pub fn use_task<D, F>(&mut self, deps: D, init: impl FnOnce(TaskContext) -> F + Send + 'static)
    where
        D: PartialEq + Send + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        let tokio_handle = self
            .tokio_handle
            .clone()
            .expect("use_task requires an app tokio runtime");
        let scheduler = self.scheduler.clone();
        let owner = self.owner;
        let mut next_deps = Some(deps);
        let mut next_init = Some(init);
        let (_, state) = self.storage.next_slot(|hook_index| {
            let deps = next_deps
                .take()
                .expect("task deps should be available for new hook slot");
            let init = next_init
                .take()
                .expect("task init should be available for new hook slot");
            let generation = 1;
            scheduler.set_async_scope(owner, hook_index, generation);
            let handle = spawn_task(&tokio_handle, owner, hook_index, generation, init);
            TaskHookState {
                deps,
                hook_index,
                generation,
                handle: Some(handle),
            }
        });

        if let Some(deps) = next_deps.take() {
            let should_restart = read_slot(state).deps != deps;
            if should_restart {
                let init = next_init
                    .take()
                    .expect("task init should be available when deps change");
                write_slot(state, |state| {
                    if let Some(handle) = state.handle.take() {
                        handle.abort();
                    }
                    state.deps = deps;
                    state.generation = state.generation.saturating_add(1);
                    scheduler.set_async_scope(owner, state.hook_index, state.generation);
                    state.handle = Some(spawn_task(
                        &tokio_handle,
                        owner,
                        state.hook_index,
                        state.generation,
                        init,
                    ));
                });
            }
        }
    }

    pub fn use_resource<D, T, E, F>(
        &mut self,
        deps: D,
        init: impl FnOnce(ResourceContext) -> F + Send + 'static,
    ) -> Resource<T, E>
    where
        D: PartialEq + Send + 'static,
        T: Send + Clone + 'static,
        E: Send + Clone + 'static,
        F: Future<Output = Result<T, E>> + Send + 'static,
    {
        let tokio_handle = self
            .tokio_handle
            .clone()
            .expect("use_resource requires an app tokio runtime");
        let dispatcher = self.dispatcher.clone();
        let owner = self.owner;
        let mut next_value_index = None;
        let (value_index, value) = self.storage.next_slot(|hook_index| {
            next_value_index = Some(hook_index);
            ResourceValueSlot {
                value: AsyncValue::Pending,
                generation: 1,
            }
        });
        self.scheduler
            .apply_hook_updates(self.owner, value_index, self.render_lanes, &value);

        let mut next_deps = Some(deps);
        let mut next_init = Some(init);
        let (_, state) = self.storage.next_slot(|_| {
            let deps = next_deps
                .take()
                .expect("resource deps should be available for new hook slot");
            let init = next_init
                .take()
                .expect("resource init should be available for new hook slot");
            let value_index =
                next_value_index.expect("resource value hook index should be initialized");
            let generation = read_slot(value).generation;
            let handle = spawn_resource(
                &tokio_handle,
                dispatcher.clone(),
                owner,
                value_index,
                generation,
                init,
            );
            ResourceHookState {
                deps,
                handle: Some(handle),
            }
        });

        if let Some(deps) = next_deps.take() {
            let should_restart = read_slot(state).deps != deps;
            if should_restart {
                let init = next_init
                    .take()
                    .expect("resource init should be available when deps change");
                let mut generation = 0;
                write_slot(value, |value| {
                    value.generation = value.generation.saturating_add(1);
                    value.value = AsyncValue::Pending;
                    generation = value.generation;
                });
                write_slot(state, |state| {
                    if let Some(handle) = state.handle.take() {
                        handle.abort();
                    }
                    state.deps = deps;
                    state.handle = Some(spawn_resource(
                        &tokio_handle,
                        dispatcher,
                        owner,
                        value_index,
                        generation,
                        init,
                    ));
                });
            }
        }

        Resource { inner: value }
    }
}

pub struct TaskContext {
    scope: AsyncScope,
}

impl TaskContext {
    pub fn scoped_setter<T: Send + 'static>(&self, setter: StateSetter<T>) -> StateSetter<T> {
        setter.with_scope(self.scope)
    }
}

pub struct ResourceContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncValue<T, E> {
    Pending,
    Ready(T),
    Error(E),
}

struct ResourceValueSlot<T, E> {
    value: AsyncValue<T, E>,
    generation: u64,
}

pub struct Resource<T: Clone + 'static, E: Clone + 'static> {
    inner: Pointer<Slot, ResourceValueSlot<T, E>>,
}

impl<T: Clone + 'static, E: Clone + 'static> Copy for Resource<T, E> {}

impl<T: Clone + 'static, E: Clone + 'static> Clone for Resource<T, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + 'static, E: Clone + 'static> Resource<T, E> {
    pub fn get(&self) -> AsyncValue<T, E> {
        read_slot(self.inner).value.clone()
    }
}

struct TaskHookState<D> {
    deps: D,
    hook_index: usize,
    generation: u64,
    handle: Option<JoinHandle<()>>,
}

impl<D> Drop for TaskHookState<D> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

struct ResourceHookState<D> {
    deps: D,
    handle: Option<JoinHandle<()>>,
}

impl<D> Drop for ResourceHookState<D> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

fn spawn_task<F>(
    tokio_handle: &TokioHandle,
    owner: FiberId,
    hook_index: usize,
    generation: u64,
    init: impl FnOnce(TaskContext) -> F + Send + 'static,
) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let ctx = TaskContext {
        scope: AsyncScope {
            owner,
            hook_index,
            generation,
        },
    };
    tokio_handle.spawn(init(ctx))
}

fn spawn_resource<T, E, F>(
    tokio_handle: &TokioHandle,
    dispatcher: AsyncDispatcher,
    owner: FiberId,
    value_index: usize,
    generation: u64,
    init: impl FnOnce(ResourceContext) -> F + Send + 'static,
) -> JoinHandle<()>
where
    T: Send + Clone + 'static,
    E: Send + Clone + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    tokio_handle.spawn(async move {
        let result = init(ResourceContext).await;
        dispatcher.dispatch_hook_update::<ResourceValueSlot<T, E>>(
            owner,
            value_index,
            RETRY_LANE,
            None,
            move |slot| {
                if slot.generation != generation {
                    return;
                }
                slot.value = match result {
                    Ok(value) => AsyncValue::Ready(value),
                    Err(error) => AsyncValue::Error(error),
                };
            },
        );
    })
}

#[derive(Clone, Default)]
pub struct Scheduler {
    inner: Rc<RefCell<SchedulerState>>,
}

#[derive(Default)]
struct SchedulerState {
    lane_root: LaneRoot,
    dirty_components: HashMap<FiberId, Lanes>,
    hook_updates: HashMap<HookKey, VecDeque<HookUpdate>>,
    async_scopes: HashMap<HookKey, u64>,
    mounted_components: HashSet<FiberId>,
    root: Option<FiberId>,
}

struct HookUpdate {
    lane: Lane,
    apply: HookApply,
}

impl Scheduler {
    pub fn set_root(&self, id: FiberId) {
        let mut inner = self.inner.borrow_mut();
        inner.root = Some(id);
        inner.mounted_components.insert(id);
    }

    pub fn mark_mounted(&self, id: FiberId) {
        self.inner.borrow_mut().mounted_components.insert(id);
    }

    pub fn mark_unmounted(&self, id: FiberId) {
        let mut inner = self.inner.borrow_mut();
        inner.mounted_components.remove(&id);
        inner.dirty_components.remove(&id);
        inner.hook_updates.retain(|(owner, _), _| *owner != id);
        inner.async_scopes.retain(|(owner, _), _| *owner != id);
    }

    pub fn set_async_scope(&self, owner: FiberId, hook_index: usize, generation: u64) {
        let mut inner = self.inner.borrow_mut();
        if !inner.mounted_components.contains(&owner) && inner.root != Some(owner) {
            return;
        }
        inner.async_scopes.insert((owner, hook_index), generation);
    }

    pub fn mark_component_dirty(&self, id: FiberId, lane: Lane) {
        let mut inner = self.inner.borrow_mut();
        if !inner.mounted_components.contains(&id) && inner.root != Some(id) {
            return;
        }
        *inner.dirty_components.entry(id).or_insert(NO_LANES) |= lane;
        inner.lane_root.mark_root_updated(lane);
    }

    pub fn mark_root_dirty(&self, lane: Lane) {
        let root = self.inner.borrow().root;
        if let Some(root) = root {
            self.mark_component_dirty(root, lane);
        }
    }

    pub fn component_lanes(&self, id: FiberId) -> Lanes {
        self.inner
            .borrow()
            .dirty_components
            .get(&id)
            .copied()
            .unwrap_or(NO_LANES)
    }

    pub fn enqueue_hook_update<T: 'static>(
        &self,
        owner: FiberId,
        hook_index: usize,
        update: impl FnOnce(&mut T) + 'static,
    ) {
        let lane = current_update_lane();
        let mut inner = self.inner.borrow_mut();
        if !inner.mounted_components.contains(&owner) {
            return;
        }

        inner
            .hook_updates
            .entry((owner, hook_index))
            .or_default()
            .push_back(HookUpdate {
                lane,
                apply: Box::new(move |value| {
                    let state = value
                        .downcast_ref::<Pointer<Slot, StateSlot<T>>>()
                        .expect("hook state update type changed");
                    write_slot(*state, |state| update(&mut state.value));
                }),
            });
        *inner.dirty_components.entry(owner).or_insert(NO_LANES) |= lane;
        inner.lane_root.mark_root_updated(lane);
    }

    pub(crate) fn enqueue_async_message(&self, message: AsyncMessage) -> bool {
        let mut inner = self.inner.borrow_mut();
        if !inner.mounted_components.contains(&message.owner) {
            return false;
        }
        if let Some(scope) = message.scope {
            if inner
                .async_scopes
                .get(&(scope.owner, scope.hook_index))
                .copied()
                != Some(scope.generation)
            {
                return false;
            }
        }

        inner
            .hook_updates
            .entry((message.owner, message.hook_index))
            .or_default()
            .push_back(HookUpdate {
                lane: message.lane,
                apply: Box::new(move |value| (message.apply)(value)),
            });
        *inner
            .dirty_components
            .entry(message.owner)
            .or_insert(NO_LANES) |= message.lane;
        inner.lane_root.mark_root_updated(message.lane);
        true
    }

    fn apply_hook_updates<T: 'static>(
        &self,
        owner: FiberId,
        hook_index: usize,
        render_lanes: Lanes,
        value: &Pointer<Slot, T>,
    ) {
        let mut inner = self.inner.borrow_mut();
        let Some(updates) = inner.hook_updates.get_mut(&(owner, hook_index)) else {
            return;
        };

        let mut remaining = VecDeque::new();
        while let Some(update) = updates.pop_front() {
            if includes_some_lane(update.lane, render_lanes) {
                (update.apply)(value);
            } else {
                remaining.push_back(update);
            }
        }

        if remaining.is_empty() {
            inner.hook_updates.remove(&(owner, hook_index));
        } else {
            *updates = remaining;
        }
    }

    pub fn pending_lanes(&self) -> Lanes {
        self.inner.borrow().lane_root.pending_lanes
    }

    pub fn get_next_lanes(&self, wip_lanes: Lanes) -> Lanes {
        self.inner.borrow().lane_root.get_next_lanes(wip_lanes)
    }

    pub fn mark_render_finished(&self, finished_lanes: Lanes) {
        let mut inner = self.inner.borrow_mut();
        for lanes in inner.dirty_components.values_mut() {
            *lanes &= !finished_lanes;
        }
        inner.dirty_components.retain(|_, lanes| *lanes != NO_LANES);

        let remaining = inner
            .dirty_components
            .values()
            .fold(NO_LANES, |acc, lanes| acc | *lanes)
            | inner
                .hook_updates
                .values()
                .flat_map(|updates| updates.iter().map(|update| update.lane))
                .fold(NO_LANES, |acc, lane| acc | lane);
        inner
            .lane_root
            .mark_root_finished(finished_lanes, remaining);
    }

    pub fn mark_starved_lanes_as_expired(&self, now_ms: u64) {
        self.inner
            .borrow_mut()
            .lane_root
            .mark_starved_lanes_as_expired(now_ms);
    }

    pub fn is_dirty(&self) -> bool {
        self.inner.borrow().lane_root.pending_lanes != NO_LANES
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc::{Receiver, TryRecvError};
    use std::time::Duration;

    use crate::fiber::FiberArena;
    use crate::lanes::{SYNC_LANE, with_update_lane};
    use xui_interface::{DirtyFlags, EventPhase, EventRequests};

    use super::*;

    fn render_callback(
        storage: &mut HookStorage,
        owner: FiberId,
        scheduler: Scheduler,
        dep: usize,
        builds: Rc<RefCell<usize>>,
    ) -> Callback<Box<dyn FnMut() -> usize>> {
        let mut cx = HookContext::new(storage, owner, scheduler, SYNC_LANE);
        cx.use_callback(dep, move || {
            *builds.borrow_mut() += 1;
            Box::new(move || dep) as Box<dyn FnMut() -> usize>
        })
    }

    fn render_state<T: Clone + 'static>(
        storage: &mut HookStorage,
        owner: FiberId,
        scheduler: Scheduler,
        init: impl FnOnce() -> T,
    ) -> State<T> {
        let mut cx = HookContext::new(storage, owner, scheduler, SYNC_LANE);
        cx.use_state(init)
    }

    fn render_two_states(
        storage: &mut HookStorage,
        owner: FiberId,
        scheduler: Scheduler,
    ) -> (State<i32>, State<&'static str>) {
        let mut cx = HookContext::new(storage, owner, scheduler, SYNC_LANE);
        let count = cx.use_state(|| 1);
        let label = cx.use_state(|| "first");
        (count, label)
    }

    fn async_parts() -> (
        AsyncDispatcher,
        Receiver<AsyncMessage>,
        tokio::runtime::Runtime,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        (AsyncDispatcher::new(sender), receiver, runtime)
    }

    fn render_async_state<T: Clone + 'static>(
        storage: &mut HookStorage,
        owner: FiberId,
        scheduler: Scheduler,
        dispatcher: AsyncDispatcher,
        init: impl FnOnce() -> T,
    ) -> State<T> {
        let mut cx = HookContext::new_with_async(
            storage,
            owner,
            scheduler,
            SYNC_LANE | DEFAULT_LANE,
            dispatcher,
            None,
        );
        cx.use_state(init)
    }

    #[test]
    fn use_state_handle_is_copy_and_reads_initial_value() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);

        let state = render_state(&mut storage, owner, scheduler, || 7);
        let copied = state;

        assert_eq!(state, copied);
        assert_eq!(*copied.get(), 7);
    }

    #[test]
    fn use_state_applies_queued_update_on_next_render() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);

        let first = render_state(&mut storage, owner, scheduler.clone(), || 1);
        with_update_lane(SYNC_LANE, || first.set(2));

        assert_eq!(*first.get(), 1);

        let second = render_state(&mut storage, owner, scheduler, || 99);

        assert_eq!(*second.get(), 2);
        assert_eq!(*first.get(), 2);
    }

    #[test]
    fn use_state_updates_are_keyed_by_hook_index() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);

        let (count, label) = render_two_states(&mut storage, owner, scheduler.clone());
        with_update_lane(SYNC_LANE, || {
            count.update(|value| *value += 4);
            label.set("second");
        });

        let (next_count, next_label) = render_two_states(&mut storage, owner, scheduler);

        assert_eq!(*next_count.get(), 5);
        assert_eq!(*next_label.get(), "second");
    }

    #[test]
    fn state_setter_enqueues_async_update() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);
        let (dispatcher, receiver, _runtime) = async_parts();

        let state = render_async_state(
            &mut storage,
            owner,
            scheduler.clone(),
            dispatcher.clone(),
            || 1,
        );
        state.setter().set(5);

        let message = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(scheduler.enqueue_async_message(message));

        let state = render_async_state(&mut storage, owner, scheduler, dispatcher, || 0);
        assert_eq!(*state.get(), 5);
    }

    #[test]
    fn use_task_runs_background_future_and_updates_state() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);
        let (dispatcher, receiver, runtime) = async_parts();

        let mut cx = HookContext::new_with_async(
            &mut storage,
            owner,
            scheduler.clone(),
            SYNC_LANE | DEFAULT_LANE,
            dispatcher.clone(),
            Some(runtime.handle().clone()),
        );
        let state = cx.use_state(|| 0);
        let setter = state.setter();
        cx.use_task(1usize, move |task| {
            let setter = task.scoped_setter(setter);
            async move {
                setter.set(7);
            }
        });

        let message = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(scheduler.enqueue_async_message(message));

        let state = render_async_state(&mut storage, owner, scheduler, dispatcher, || 0);
        assert_eq!(*state.get(), 7);
    }

    #[test]
    fn use_resource_moves_from_pending_to_ready() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);
        let (dispatcher, receiver, runtime) = async_parts();

        let mut cx = HookContext::new_with_async(
            &mut storage,
            owner,
            scheduler.clone(),
            SYNC_LANE | RETRY_LANE,
            dispatcher.clone(),
            Some(runtime.handle().clone()),
        );
        let resource = cx.use_resource(1usize, |_| async move { Ok::<_, ()>(9usize) });
        assert_eq!(resource.get(), AsyncValue::Pending);

        let message = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(scheduler.enqueue_async_message(message));

        let mut cx = HookContext::new_with_async(
            &mut storage,
            owner,
            scheduler,
            SYNC_LANE | RETRY_LANE,
            dispatcher,
            Some(runtime.handle().clone()),
        );
        let resource = cx.use_resource(1usize, |_| async move { Ok::<_, ()>(0usize) });
        assert_eq!(resource.get(), AsyncValue::Ready(9));
        assert!(matches!(
            receiver.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn use_callback_reuses_handle_when_deps_are_equal() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);
        let builds = Rc::new(RefCell::new(0));

        let first = render_callback(&mut storage, owner, scheduler.clone(), 7, builds.clone());
        let second = render_callback(&mut storage, owner, scheduler, 7, builds.clone());

        assert!(first.ptr_eq(&second));
        assert_eq!(*builds.borrow(), 1);
        assert_eq!(first.call_mut(|callback| callback()), 7);
        assert_eq!(second.call_mut(|callback| callback()), 7);
    }

    #[test]
    fn use_callback_updates_callback_when_deps_change() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);
        let builds = Rc::new(RefCell::new(0));

        let first = render_callback(&mut storage, owner, scheduler.clone(), 7, builds.clone());
        let second = render_callback(&mut storage, owner, scheduler, 9, builds.clone());

        assert!(first.ptr_eq(&second));
        assert_eq!(*builds.borrow(), 2);
        assert_eq!(first.call_mut(|callback| callback()), 9);
        assert_eq!(second.call_mut(|callback| callback()), 9);
    }

    #[test]
    fn use_callback_can_store_event_handlers() {
        let mut storage = HookStorage::default();
        let scheduler = Scheduler::default();
        let owner = FiberArena::new().root();
        scheduler.set_root(owner);
        let mut cx = HookContext::new(&mut storage, owner, scheduler, SYNC_LANE);

        let callback = cx.use_callback((), || {
            Box::new(|cx: &mut xui_interface::events::EventContext<'_>| {
                cx.mark_needs_paint();
                xui_interface::events::EventResult::Consumed
            })
                as Box<
                    dyn for<'a> FnMut(
                        &mut xui_interface::events::EventContext<'a>,
                    ) -> xui_interface::events::EventResult,
                >
        });

        let mut dirty = DirtyFlags::empty();
        let mut requests = EventRequests::default();
        let mut event_cx = xui_interface::events::EventContext::new(
            Default::default(),
            EventPhase::Target,
            &mut dirty,
            &mut requests,
        );
        let result = callback.call_mut(|handler| handler(&mut event_cx));

        assert_eq!(result, xui_interface::events::EventResult::Consumed);
        assert!(dirty.contains(DirtyFlags::PAINT));
    }
}
