//! `kept!(state.transition(args))`: calls a transition that answers `(next, out)`, keeps `next`
//! in `state`, and gives `out`, so tests read as the old in-place calls did.

#[allow(unused_macros)]
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

/// `stepped!(state.transition(args))`: as `kept!`, for a transition that answers
/// `Result<(next, out), _>`; a refusal leaves `state` as it was.
#[allow(unused_macros)]
macro_rules! stepped {
    ($state:ident . $transition:ident ( $($arg:expr),* $(,)? )) => {{
        $state.$transition($($arg),*).map(|(next, out)| {
            #[allow(unused_assignments)]
            {
                $state = next;
            }
            out
        })
    }};
}
