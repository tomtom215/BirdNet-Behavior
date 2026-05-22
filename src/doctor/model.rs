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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    #[test]
    fn skip_when_unconfigured() {
        let checks = check_model(&cli(), None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Skip);
    }

    #[test]
    fn pass_for_large_model_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, vec![0u8; 1_000_001]).unwrap();
        let mut cli = cli();
        cli.model = Some(model);
        let checks = check_model(&cli, None);
        assert_eq!(checks[0].status, Status::Pass);
        assert!(checks[0].name.contains("ONNX model"));
    }

    #[test]
    fn warn_for_tiny_model_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, b"tiny").unwrap();
        let mut cli = cli();
        cli.model = Some(model);
        let checks = check_model(&cli, None);
        assert_eq!(checks[0].status, Status::Warn);
        assert!(checks[0].message.contains("truncated"));
    }

    #[test]
    fn fail_for_missing_model_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.model = Some(dir.path().join("absent.onnx"));
        let checks = check_model(&cli, None);
        assert_eq!(checks[0].status, Status::Fail);
        assert!(checks[0].message.contains("does not exist"));
    }

    #[test]
    fn labels_pass_when_present_and_fail_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("labels.txt");
        std::fs::write(&labels, "Turdus merula_Common Blackbird").unwrap();
        let mut cli_present = cli();
        cli_present.labels = Some(labels);
        let checks = check_model(&cli_present, None);
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Labels") && c.status == Status::Pass)
        );

        let mut cli_absent = cli();
        cli_absent.labels = Some(dir.path().join("absent-labels.txt"));
        let checks = check_model(&cli_absent, None);
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Labels") && c.status == Status::Fail)
        );
    }
}
