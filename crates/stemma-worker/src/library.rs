//! Builds of a group's library, and their inspection.

use stemma_core::protocol::{JobSpec, LibraryBuild, Outcome};

use crate::job::{Context, Progress};

pub async fn build(
    _context: &Context,
    _job: &JobSpec,
    _build: &LibraryBuild,
    _progress: &Progress,
) -> anyhow::Result<Outcome> {
    anyhow::bail!("this worker does not build libraries yet")
}
