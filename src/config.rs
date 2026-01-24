use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "ralph")]
#[command(about = "Autonomous AI agent loop for iterative development")]
#[command(version)]
pub struct Args {
    /// Path to PRD file
    #[arg(short, long, default_value = "prd.jsonc")]
    pub prd: PathBuf,

    /// Path to custom system prompt file (uses built-in if not specified)
    #[arg(short = 'P', long)]
    pub prompt: Option<PathBuf>,

    /// Maximum iterations (0 = unlimited)
    #[arg(short = 'm', long, default_value_t = 10)]
    pub max_iterations: u32,

    /// Delay between iterations in seconds
    #[arg(short, long, default_value_t = 2)]
    pub delay: u64,

    /// Completion marker text (overrides PRD)
    #[arg(short, long)]
    pub completion_marker: Option<String>,

    /// Claude permission mode: default, acceptEdits, plan
    #[arg(long, default_value = "acceptEdits")]
    pub permission_mode: String,

    /// Use --continue mode (preserves session context)
    #[arg(long)]
    pub continue_session: bool,

    /// Skip all permission prompts
    #[arg(long)]
    pub dangerously_skip_permissions: bool,

    /// Skip initialization phase
    #[arg(long)]
    pub skip_init: bool,

    /// Initialize a new prd.jsonc template
    #[arg(long)]
    pub init: bool,

    /// Initialize a new custom prompt template
    #[arg(long)]
    pub init_prompt: bool,

    /// Timeout per Claude execution in seconds
    #[arg(short = 't', long, default_value_t = 1800)]
    pub timeout: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    mod default_values {
        use super::*;

        fn parse_args(args: &[&str]) -> Args {
            Args::try_parse_from(std::iter::once("ralph").chain(args.iter().copied())).unwrap()
        }

        #[test]
        fn prd_defaults_to_prd_jsonc() {
            let args = parse_args(&[]);
            assert_eq!(args.prd, PathBuf::from("prd.jsonc"));
        }

        #[test]
        fn max_iterations_defaults_to_10() {
            let args = parse_args(&[]);
            assert_eq!(args.max_iterations, 10);
        }

        #[test]
        fn delay_defaults_to_2() {
            let args = parse_args(&[]);
            assert_eq!(args.delay, 2);
        }

        #[test]
        fn completion_marker_defaults_to_none() {
            let args = parse_args(&[]);
            assert!(args.completion_marker.is_none());
        }

        #[test]
        fn permission_mode_defaults_to_accept_edits() {
            let args = parse_args(&[]);
            assert_eq!(args.permission_mode, "acceptEdits");
        }

        #[test]
        fn continue_session_defaults_to_false() {
            let args = parse_args(&[]);
            assert!(!args.continue_session);
        }

        #[test]
        fn dangerously_skip_permissions_defaults_to_false() {
            let args = parse_args(&[]);
            assert!(!args.dangerously_skip_permissions);
        }

        #[test]
        fn skip_init_defaults_to_false() {
            let args = parse_args(&[]);
            assert!(!args.skip_init);
        }

        #[test]
        fn init_defaults_to_false() {
            let args = parse_args(&[]);
            assert!(!args.init);
        }

        #[test]
        fn init_prompt_defaults_to_false() {
            let args = parse_args(&[]);
            assert!(!args.init_prompt);
        }

        #[test]
        fn timeout_defaults_to_1800() {
            let args = parse_args(&[]);
            assert_eq!(args.timeout, 1800);
        }

        #[test]
        fn prompt_defaults_to_none() {
            let args = parse_args(&[]);
            assert!(args.prompt.is_none());
        }
    }

    mod argument_overrides {
        use super::*;

        fn parse_args(args: &[&str]) -> Args {
            Args::try_parse_from(std::iter::once("ralph").chain(args.iter().copied())).unwrap()
        }

        #[test]
        fn prd_short_flag() {
            let args = parse_args(&["-p", "custom.json"]);
            assert_eq!(args.prd, PathBuf::from("custom.json"));
        }

        #[test]
        fn prd_long_flag() {
            let args = parse_args(&["--prd", "custom.json"]);
            assert_eq!(args.prd, PathBuf::from("custom.json"));
        }

        #[test]
        fn max_iterations_short_flag() {
            let args = parse_args(&["-m", "50"]);
            assert_eq!(args.max_iterations, 50);
        }

        #[test]
        fn max_iterations_long_flag() {
            let args = parse_args(&["--max-iterations", "100"]);
            assert_eq!(args.max_iterations, 100);
        }

        #[test]
        fn max_iterations_zero_means_unlimited() {
            let args = parse_args(&["-m", "0"]);
            assert_eq!(args.max_iterations, 0);
        }

        #[test]
        fn delay_short_flag() {
            let args = parse_args(&["-d", "5"]);
            assert_eq!(args.delay, 5);
        }

        #[test]
        fn delay_long_flag() {
            let args = parse_args(&["--delay", "10"]);
            assert_eq!(args.delay, 10);
        }

        #[test]
        fn completion_marker_short_flag() {
            let args = parse_args(&["-c", "DONE"]);
            assert_eq!(args.completion_marker, Some("DONE".to_string()));
        }

        #[test]
        fn completion_marker_long_flag() {
            let args = parse_args(&["--completion-marker", "<FINISHED>"]);
            assert_eq!(args.completion_marker, Some("<FINISHED>".to_string()));
        }

        #[test]
        fn permission_mode_override() {
            let args = parse_args(&["--permission-mode", "plan"]);
            assert_eq!(args.permission_mode, "plan");
        }

        #[test]
        fn continue_session_flag() {
            let args = parse_args(&["--continue-session"]);
            assert!(args.continue_session);
        }

        #[test]
        fn dangerously_skip_permissions_flag() {
            let args = parse_args(&["--dangerously-skip-permissions"]);
            assert!(args.dangerously_skip_permissions);
        }

        #[test]
        fn skip_init_flag() {
            let args = parse_args(&["--skip-init"]);
            assert!(args.skip_init);
        }

        #[test]
        fn init_flag() {
            let args = parse_args(&["--init"]);
            assert!(args.init);
        }

        #[test]
        fn init_prompt_flag() {
            let args = parse_args(&["--init-prompt"]);
            assert!(args.init_prompt);
        }

        #[test]
        fn timeout_short_flag() {
            let args = parse_args(&["-t", "3600"]);
            assert_eq!(args.timeout, 3600);
        }

        #[test]
        fn timeout_long_flag() {
            let args = parse_args(&["--timeout", "600"]);
            assert_eq!(args.timeout, 600);
        }

        #[test]
        fn prompt_short_flag() {
            let args = parse_args(&["-P", "custom-prompt.md"]);
            assert_eq!(args.prompt, Some(PathBuf::from("custom-prompt.md")));
        }

        #[test]
        fn prompt_long_flag() {
            let args = parse_args(&["--prompt", "my-prompt.txt"]);
            assert_eq!(args.prompt, Some(PathBuf::from("my-prompt.txt")));
        }
    }

    mod edge_cases {
        use super::*;

        fn parse_args(args: &[&str]) -> Args {
            Args::try_parse_from(std::iter::once("ralph").chain(args.iter().copied())).unwrap()
        }

        fn try_parse_args(args: &[&str]) -> Result<Args, clap::Error> {
            Args::try_parse_from(std::iter::once("ralph").chain(args.iter().copied()))
        }

        #[test]
        fn prd_path_with_spaces() {
            let args = parse_args(&["-p", "path with spaces/prd.json"]);
            assert_eq!(args.prd, PathBuf::from("path with spaces/prd.json"));
        }

        #[test]
        fn prd_path_absolute() {
            let args = parse_args(&["-p", "/home/user/project/prd.jsonc"]);
            assert_eq!(args.prd, PathBuf::from("/home/user/project/prd.jsonc"));
        }

        #[test]
        fn completion_marker_with_special_chars() {
            let args = parse_args(&["-c", "<promise>COMPLETE</promise>"]);
            assert_eq!(
                args.completion_marker,
                Some("<promise>COMPLETE</promise>".to_string())
            );
        }

        #[test]
        fn completion_marker_empty_string() {
            let args = parse_args(&["-c", ""]);
            assert_eq!(args.completion_marker, Some(String::new()));
        }

        #[test]
        fn multiple_flags_combined() {
            let args = parse_args(&[
                "-p",
                "custom.json",
                "-m",
                "20",
                "-d",
                "5",
                "-t",
                "600",
                "--continue-session",
                "--skip-init",
            ]);
            assert_eq!(args.prd, PathBuf::from("custom.json"));
            assert_eq!(args.max_iterations, 20);
            assert_eq!(args.delay, 5);
            assert_eq!(args.timeout, 600);
            assert!(args.continue_session);
            assert!(args.skip_init);
        }

        #[test]
        fn invalid_max_iterations_non_numeric() {
            let result = try_parse_args(&["-m", "abc"]);
            assert!(result.is_err());
        }

        #[test]
        fn invalid_delay_non_numeric() {
            let result = try_parse_args(&["-d", "xyz"]);
            assert!(result.is_err());
        }

        #[test]
        fn invalid_timeout_non_numeric() {
            let result = try_parse_args(&["-t", "not_a_number"]);
            assert!(result.is_err());
        }

        #[test]
        fn invalid_timeout_negative() {
            let result = try_parse_args(&["-t", "-1"]);
            assert!(result.is_err());
        }

        #[test]
        fn unknown_flag_rejected() {
            let result = try_parse_args(&["--unknown-flag"]);
            assert!(result.is_err());
        }

        #[test]
        fn permission_mode_accepts_any_string() {
            let args = parse_args(&["--permission-mode", "customMode"]);
            assert_eq!(args.permission_mode, "customMode");
        }

        #[test]
        fn large_max_iterations() {
            let args = parse_args(&["-m", "4294967295"]);
            assert_eq!(args.max_iterations, u32::MAX);
        }

        #[test]
        fn large_timeout() {
            let args = parse_args(&["-t", "18446744073709551615"]);
            assert_eq!(args.timeout, u64::MAX);
        }

        #[test]
        fn delay_zero() {
            let args = parse_args(&["-d", "0"]);
            assert_eq!(args.delay, 0);
        }

        #[test]
        fn timeout_zero() {
            let args = parse_args(&["-t", "0"]);
            assert_eq!(args.timeout, 0);
        }

        #[test]
        fn prompt_path_with_spaces() {
            let args = parse_args(&["-P", "path with spaces/prompt.md"]);
            assert_eq!(args.prompt, Some(PathBuf::from("path with spaces/prompt.md")));
        }

        #[test]
        fn prompt_path_absolute() {
            let args = parse_args(&["-P", "/home/user/prompts/custom.md"]);
            assert_eq!(args.prompt, Some(PathBuf::from("/home/user/prompts/custom.md")));
        }
    }
}
