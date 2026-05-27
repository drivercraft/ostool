use ostool::invocation::{ActiveBuildContext, InvocationState};

fn main() {
    let _ = core::mem::size_of::<InvocationState>();
    let _ = core::mem::size_of::<ActiveBuildContext>();
}
