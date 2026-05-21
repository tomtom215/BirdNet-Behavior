//! Model checks: ONNX model file presence/size and labels file presence.

use std::path::PathBuf;

use birdnet_core::config::Config;

use super::Check;
use crate::cli::Cli;

pub(super) fn check_model(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let model_path = cli
        .model
        .clone()
        .or_else(|| config?.get("MODEL").map(PathBuf::from));
    let mut out = Vec::new();

    if let Some(p) = model_path {
        if p.exists() {
            match std::fs::metadata(&p) {
                Ok(m) if m.len() > 1_000_000 => out.push(Check::pass(
                    "ONNX model file",
                    format!("{} ({} bytes)", p.display(), m.len()),
                )),
                Ok(m) => out.push(Check::warn(
                    "ONNX model file",
                    format!(
                        "{} is only {} bytes — likely truncated or empty",
                        p.display(),
                        m.len()
                    ),
                    "re-download the model (delete it; the entrypoint will fetch it again)",
                )),
                Err(e) => out.push(Check::fail(
                    "ONNX model file",
                    format!("{} could not be inspected: {e}", p.display()),
                    "check filesystem health and permissions",
                )),
            }
        } else {
            out.push(Check::fail(
                "ONNX model file",
                format!("{} does not exist", p.display()),
                "either let the entrypoint download it (Docker), or run `install.sh` again",
            ));
        }
    } else {
        out.push(Check::skip(
            "ONNX model file",
            "no --model / MODEL configured (will use the bundled default at startup)",
        ));
    }

    let labels_path = cli
        .labels
        .clone()
        .or_else(|| config?.get("LABELS").map(PathBuf::from));
    if let Some(p) = labels_path {
        if p.exists() {
            out.push(Check::pass(
                "Labels file",
                format!("{} exists", p.display()),
            ));
        } else {
            out.push(Check::fail(
                "Labels file",
                format!("{} does not exist", p.display()),
                "the labels file ships alongside the model; re-run `install.sh`",
            ));
        }
    }

    out
}
