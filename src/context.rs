use crate::config::DockConfig;
use crate::state::DockState;
use nwg_common::compositor::Compositor;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Shared context passed to all dock UI builders.
///
/// Bundles the commonly-needed references so functions don't need 5+ parameters.
pub struct DockContext {
    pub config: Rc<DockConfig>,
    pub state: Rc<RefCell<DockState>>,
    pub data_home: Rc<PathBuf>,
    pub pinned_file: Rc<PathBuf>,
    pub rebuild: Rc<dyn Fn()>,
    pub compositor: Rc<dyn Compositor>,
}
