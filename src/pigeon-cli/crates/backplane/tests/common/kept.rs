//! `kept!(state.transition(args))`: calls a transition that answers `(next, out)`, keeps `next`
//! in `state`, and gives `out`, so tests read as the old in-place calls did.

macro_rules! kept {
    ($state:ident . $transition:ident ( $($arg:expr),* $(,)? )) => {{
        let (next, out) = $state.$transition($($arg),*);
        #[allow(unused_assignments)]
        {
            $state = next;
        }
        out
    }};
}
