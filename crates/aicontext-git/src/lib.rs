//! A narrow, safe wrapper over git.
//!
//! The platform is Git-native: knowledge is committed, branched, reviewed, and diffed like code. This
//! crate is the only place allowed to invoke `git`, and it invokes it deliberately.
//!
//! # Safety rules
//!
//! - Arguments are always passed as an argv array. Nothing is ever formatted into a shell string,
//!   so no branch name, path, or commit message can be interpreted as a command.
//! - Read operations only. Committing, pushing, branching, and history rewriting are human actions
//!   and are deliberately not implemented here.
//! - Every failure is a typed error naming the git exit status, because a bare "git failed" costs
//!   the reader an hour.
//!
//! # What it answers
//!
//! | Question | Method |
//! |----------|--------|
//! | Is this a repository at all? | [`Git::is_repository`] |
//! | Which branch, and what changed? | [`Git::open`] then [`Repository::snapshot`] |
//! | The branch name alone | [`Repository::branch`] |
//! | Changed files alone | [`Repository::changed_files`] |
//! | Recent commits | [`Repository::recent_commits`] |
//!
//! # The three states that are not failures
//!
//! A repository that does not exist, one with no commits, and one that is detached from every branch
//! are all ordinary situations, and a tool that treats them as errors cannot render them. Each has
//! its own type so a caller can handle it deliberately:
//!
//! - [`Git::is_repository`] returns `false` outside a working tree; [`Git::open`] returns
//!   [`GitError::NotARepository`].
//! - [`Head::Unborn`] is a branch with a name and no commits. [`Head::Detached`] is a commit with no
//!   branch. Reporting both as "no branch" would be false about one of them.
//! - [`Repository::recent_commits`] on an empty repository returns [`GitError::NoCommits`], because a
//!   log is a question with no answer yet rather than a broken one.
//!
//! # Example
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use aicontext_git::{Git, Head};
//!
//! // Any directory will do; this one always exists, which is why the example can actually run.
//! let root = std::env::current_dir()?;
//!
//! match Git::new(&root).open() {
//!     Ok(repository) => {
//!         let snapshot = repository.snapshot()?;
//!         match &snapshot.head {
//!             Head::Branch { name, commit } => {
//!                 println!("on {name} at {commit}, {} changed", snapshot.changes.len());
//!             }
//!             Head::Unborn { name } => println!("on {name}, nothing committed yet"),
//!             Head::Detached { commit } => println!("detached at {commit}"),
//!             // `Head` is `#[non_exhaustive]`, so a future variant is not a compile error here.
//!             _ => println!("on a head this version does not name"),
//!         }
//!     }
//!     // Outside a repository is a normal answer, not a crash: `aicontext status` says so.
//!     Err(error) if error.code() == aicontext_core::ErrorCode::GIT_002 => {
//!         println!("not a git repository");
//!     }
//!     Err(error) => return Err(error.into()),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # What this crate deliberately does not do
//!
//! It never writes. There is no `commit`, `checkout`, `fetch`, `push`, or `reset`, and the argv array
//! is fixed inside each method rather than accepted from a caller, so there is no argument a caller can
//! pass that turns a read into a write. That is the mechanical form of "Git-native, human-owned": the
//! tool reads the repository and a person changes it.
//!
//! It also does not cache. Two consecutive calls see two consecutive states, because a caller that
//! writes a file between them must see the change; caching would be faster and wrong.

#![forbid(unsafe_code)]

mod error;
mod git;
mod porcelain;
mod query;

pub use error::GitError;
pub use git::Git;
pub use porcelain::{Change, FileStatus, Head};
pub use query::{Commit, Repository, Snapshot};
