//! JCL-lite: parse JCL and run jobs the way JES2 and the z/OS initiator do,
//! against a local directory that stands in for DASD, the catalog and the spool.

pub mod catalog;
pub mod jes;
pub mod parse;
pub mod system;
mod utilities;

pub use catalog::{Catalog, DatasetEntry, GdgBase};
pub use jes::{DdRecord, JobRecord, JobStatus, SpoolFile, StepOutcome, StepRecord};
pub use parse::{JclError, Job, parse_job};
pub use system::{DatasetInfo, System};
