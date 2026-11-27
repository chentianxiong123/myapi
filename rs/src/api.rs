//! 面板 API（配置/节点管理）

//! Web API（对应 web/app.py）：CPA 节点管理 + 配置读写
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;




mod io;
pub(crate) use io::*;
mod config_api;
pub(crate) use config_api::*;
mod mgmt_api;
pub(crate) use mgmt_api::*;

#[cfg(test)]
mod tests;