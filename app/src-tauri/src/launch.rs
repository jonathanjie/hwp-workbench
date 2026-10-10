//! Decide process mode before any native app state is created.
use std::{ffi::OsString, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    Worker,
    Cli,
    Desktop,
}

pub(crate) fn launch_mode(argv0: &Path, args: &[OsString]) -> LaunchMode {
    if args.first().is_some_and(|argument| argument == "__rhwp-worker") {
        LaunchMode::Worker
    } else if argv0.file_name().is_some_and(|name| name == "hwp-workbench") {
        LaunchMode::Cli
    } else {
        LaunchMode::Desktop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, path::Path};

    #[test]
    fn exact_cli_basename_runs_the_cli() {
        for name in [
            "hwp-workbench",
            "/tmp/hwp-workbench",
            "/Applications/HWP Workbench.app/Contents/Resources/bin/hwp-workbench",
        ] {
            assert_eq!(
                launch_mode(Path::new(name), &[OsString::from("--version")]),
                LaunchMode::Cli
            );
        }
    }

    #[test]
    fn all_other_names_keep_native_open_and_single_instance_arguments() {
        for name in [
            "HWP Workbench",
            "hwp-workbench-app",
            "hwp-workbench.exe",
            "hwp-workbench-other",
            "",
        ] {
            for args in [
                vec![],
                vec![OsString::from("--version")],
                vec![OsString::from("--open"), OsString::from("/tmp/form.hwpx")],
                vec![OsString::from("/tmp/form.hwpx")],
            ] {
                assert_eq!(launch_mode(Path::new(name), &args), LaunchMode::Desktop);
            }
        }
    }

    #[test]
    fn worker_mode_takes_priority_over_executable_name() {
        for name in ["HWP Workbench", "hwp-workbench-app", "hwp-workbench", ""] {
            assert_eq!(
                launch_mode(Path::new(name), &[OsString::from("__rhwp-worker")]),
                LaunchMode::Worker
            );
        }
        assert_eq!(
            launch_mode(
                Path::new("HWP Workbench"),
                &[OsString::from("--open"), OsString::from("__rhwp-worker")]
            ),
            LaunchMode::Desktop
        );
    }
}
