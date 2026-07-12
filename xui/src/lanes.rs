use std::cell::Cell;

use xui_interface::events::RawEvent;

pub type Lane = u32;
pub type Lanes = u32;

pub const NO_LANE: Lane = 0;
pub const NO_LANES: Lanes = 0;

pub const SYNC_LANE: Lane = 0b0000_0000_0000_0010;
pub const INPUT_CONTINUOUS_LANE: Lane = 0b0000_0000_0000_1000;
pub const DEFAULT_LANE: Lane = 0b0000_0000_0010_0000;

pub const TRANSITION_LANE_1: Lane = 0b0000_0001_0000_0000;
pub const TRANSITION_LANE_2: Lane = 0b0000_0010_0000_0000;
pub const TRANSITION_LANE_3: Lane = 0b0000_0100_0000_0000;
pub const TRANSITION_LANE_4: Lane = 0b0000_1000_0000_0000;
pub const TRANSITION_LANES: Lanes =
    TRANSITION_LANE_1 | TRANSITION_LANE_2 | TRANSITION_LANE_3 | TRANSITION_LANE_4;

pub const RETRY_LANE: Lane = 0b0001_0000_0000_0000;
pub const IDLE_LANE: Lane = 0b0010_0000_0000_0000;

const SYNC_UPDATE_LANES: Lanes = SYNC_LANE | INPUT_CONTINUOUS_LANE | DEFAULT_LANE;

thread_local! {
    static CURRENT_UPDATE_LANE: Cell<Lane> = const { Cell::new(DEFAULT_LANE) };
    static NEXT_TRANSITION_LANE: Cell<Lane> = const { Cell::new(TRANSITION_LANE_1) };
}

pub fn current_update_lane() -> Lane {
    CURRENT_UPDATE_LANE.with(Cell::get)
}

pub fn with_update_lane<R>(lane: Lane, f: impl FnOnce() -> R) -> R {
    CURRENT_UPDATE_LANE.with(|current| {
        let previous = current.replace(lane);
        let output = f();
        current.set(previous);
        output
    })
}

pub fn start_transition<R>(f: impl FnOnce() -> R) -> R {
    with_update_lane(claim_next_transition_lane(), f)
}

pub fn claim_next_transition_lane() -> Lane {
    NEXT_TRANSITION_LANE.with(|next| {
        let lane = next.get();
        let following = match lane {
            TRANSITION_LANE_1 => TRANSITION_LANE_2,
            TRANSITION_LANE_2 => TRANSITION_LANE_3,
            TRANSITION_LANE_3 => TRANSITION_LANE_4,
            _ => TRANSITION_LANE_1,
        };
        next.set(following);
        lane
    })
}

pub fn event_lane(event: &RawEvent) -> Lane {
    match event {
        RawEvent::PointerMove(_) | RawEvent::Wheel(_) => INPUT_CONTINUOUS_LANE,
        RawEvent::Ime(_)
        | RawEvent::PointerDown(_)
        | RawEvent::PointerUp(_)
        | RawEvent::PointerCancel(_)
        | RawEvent::Keyboard(_)
        | RawEvent::WindowFocus(_)
        | RawEvent::WindowBlur(_)
        | RawEvent::ContextMenu(_) => SYNC_LANE,
    }
}

pub fn merge_lanes(a: Lanes, b: Lanes) -> Lanes {
    a | b
}

pub fn remove_lanes(set: Lanes, subset: Lanes) -> Lanes {
    set & !subset
}

pub fn includes_some_lane(a: Lanes, b: Lanes) -> bool {
    (a & b) != NO_LANES
}

pub fn is_subset_of_lanes(set: Lanes, subset: Lanes) -> bool {
    (set & subset) == subset
}

pub fn get_highest_priority_lane(lanes: Lanes) -> Lane {
    lanes & lanes.wrapping_neg()
}

pub fn get_highest_priority_lanes(lanes: Lanes) -> Lanes {
    let sync_lanes = lanes & SYNC_UPDATE_LANES;
    if sync_lanes != NO_LANES {
        return sync_lanes;
    }

    let lane = get_highest_priority_lane(lanes);
    if (lane & TRANSITION_LANES) != NO_LANES {
        return lanes & TRANSITION_LANES;
    }
    lane
}

pub fn is_higher_priority(a: Lane, b: Lane) -> bool {
    a != NO_LANE && (b == NO_LANE || a < b)
}

pub fn should_interrupt(wip_lanes: Lanes, next_lanes: Lanes) -> bool {
    if wip_lanes == NO_LANES || next_lanes == NO_LANES || wip_lanes == next_lanes {
        return false;
    }

    let next = get_highest_priority_lane(next_lanes);
    let wip = get_highest_priority_lane(wip_lanes);
    is_higher_priority(next, wip)
}

pub fn includes_sync_lane(lanes: Lanes) -> bool {
    includes_some_lane(lanes, SYNC_LANE)
}

#[derive(Debug, Clone)]
pub struct LaneRoot {
    pub pending_lanes: Lanes,
    pub suspended_lanes: Lanes,
    pub pinged_lanes: Lanes,
    pub expired_lanes: Lanes,
    pub entangled_lanes: Lanes,
    pub entanglements: [Lanes; 32],
    expiration_times: [Option<u64>; 32],
}

impl Default for LaneRoot {
    fn default() -> Self {
        Self {
            pending_lanes: NO_LANES,
            suspended_lanes: NO_LANES,
            pinged_lanes: NO_LANES,
            expired_lanes: NO_LANES,
            entangled_lanes: NO_LANES,
            entanglements: [NO_LANES; 32],
            expiration_times: [None; 32],
        }
    }
}

impl LaneRoot {
    pub fn mark_root_updated(&mut self, lane: Lane) {
        self.pending_lanes |= lane;
        if lane != IDLE_LANE {
            self.suspended_lanes = NO_LANES;
            self.pinged_lanes = NO_LANES;
        }
    }

    pub fn mark_root_finished(&mut self, finished_lanes: Lanes, remaining_lanes: Lanes) {
        let no_longer_pending = self.pending_lanes & !remaining_lanes;
        self.pending_lanes = remaining_lanes;
        self.suspended_lanes = NO_LANES;
        self.pinged_lanes = NO_LANES;
        self.expired_lanes &= remaining_lanes;
        self.entangled_lanes &= remaining_lanes;

        let mut lanes = no_longer_pending;
        while lanes != NO_LANES {
            let lane = get_highest_priority_lane(lanes);
            let index = lane_to_index(lane);
            self.entanglements[index] = NO_LANES;
            self.expiration_times[index] = None;
            lanes &= !lane;
        }

        self.pending_lanes &= !finished_lanes | remaining_lanes;
    }

    pub fn mark_root_entangled(&mut self, lanes: Lanes) {
        self.entangled_lanes |= lanes;
        let root_entangled = self.entangled_lanes;
        let mut cursor = root_entangled;
        while cursor != NO_LANES {
            let lane = get_highest_priority_lane(cursor);
            let index = lane_to_index(lane);
            if (lane & lanes) != NO_LANES || (self.entanglements[index] & lanes) != NO_LANES {
                self.entanglements[index] |= lanes;
            }
            cursor &= !lane;
        }
    }

    pub fn mark_starved_lanes_as_expired(&mut self, now_ms: u64) {
        let mut lanes = self.pending_lanes & !RETRY_LANE;
        while lanes != NO_LANES {
            let lane = get_highest_priority_lane(lanes);
            let index = lane_to_index(lane);
            match self.expiration_times[index] {
                Some(expires_at) if expires_at <= now_ms => {
                    self.expired_lanes |= lane;
                }
                None if (lane & self.suspended_lanes) == NO_LANES
                    || (lane & self.pinged_lanes) != NO_LANES =>
                {
                    self.expiration_times[index] = Some(now_ms + expiration_ms(lane));
                }
                _ => {}
            }
            lanes &= !lane;
        }
    }

    pub fn get_next_lanes(&self, wip_lanes: Lanes) -> Lanes {
        if self.pending_lanes == NO_LANES {
            return NO_LANES;
        }

        if self.expired_lanes != NO_LANES {
            return get_highest_priority_lanes(self.expired_lanes);
        }

        let unblocked = self.pending_lanes & !self.suspended_lanes;
        let mut next = if unblocked != NO_LANES {
            get_highest_priority_lanes(unblocked)
        } else if self.pinged_lanes != NO_LANES {
            get_highest_priority_lanes(self.pinged_lanes)
        } else {
            NO_LANES
        };

        if next == NO_LANES {
            return NO_LANES;
        }

        if wip_lanes != NO_LANES && !should_interrupt(wip_lanes, next) {
            return wip_lanes;
        }

        let entangled = self.entangled_lanes & next;
        if entangled != NO_LANES {
            let mut lanes = entangled;
            while lanes != NO_LANES {
                let lane = get_highest_priority_lane(lanes);
                next |= self.entanglements[lane_to_index(lane)];
                lanes &= !lane;
            }
        }

        next
    }
}

fn expiration_ms(lane: Lane) -> u64 {
    if (lane & (SYNC_LANE | INPUT_CONTINUOUS_LANE)) != NO_LANES {
        250
    } else if (lane & (DEFAULT_LANE | TRANSITION_LANES)) != NO_LANES {
        5_000
    } else {
        u64::MAX / 4
    }
}

fn lane_to_index(lane: Lane) -> usize {
    lane.trailing_zeros() as usize
}
