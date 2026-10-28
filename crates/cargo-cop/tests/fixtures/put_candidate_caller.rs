mod candidates;

use self::candidates::{CandidateSelection, PutSource};

struct PutOperation;

impl PutOperation {
    fn use_candidate_api<P>(_: PutSource<()>) {
        if let Ok(candidates) = CandidateSelection::candidates::<P>(loop {}, None, loop {}, loop {})
        {
            for candidate in candidates {
                let _ = candidate.layout;
                let _ = candidate.copy_on_cross_device;
            }
        }
    }
}

struct Unrelated;

impl Unrelated {
    fn new() -> Self {
        Self
    }
}
