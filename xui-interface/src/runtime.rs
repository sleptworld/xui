pub trait EventSource {
    type Event;

    fn poll_event(&mut self) -> Option<Self::Event>;
}
