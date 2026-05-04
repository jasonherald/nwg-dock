use super::schema::RawConfigFile;
use crate::config::DockConfig;

/// Merges precedence: CLI explicit > file > CLI default.
///
/// For each field, asks `matches.value_source(field_id)` whether the
/// value in `cli` came from the command line. If so, it stays.
/// Otherwise, if `file` has `Some(_)` for that field, the file value
/// replaces the CLI default. Otherwise the CLI default stands.
///
/// `field_id` for clap is the snake_case form of the field — e.g.,
/// `--icon-size` → `"icon_size"`. Bool flags (no value) follow the same
/// API; presence of `--autohide` on the CLI returns
/// `ValueSource::CommandLine`.
pub(crate) fn merge(
    matches: &clap::ArgMatches,
    mut cli: DockConfig,
    file: Option<RawConfigFile>,
) -> DockConfig {
    let Some(file) = file else {
        return cli;
    };

    macro_rules! overlay {
        ($field:ident, $id:literal, $file_value:expr) => {
            if !was_set_on_cli(matches, $id)
                && let Some(v) = $file_value
            {
                cli.$field = v;
            }
        };
    }

    // [behavior]
    overlay!(autohide, "autohide", file.behavior.autohide);
    overlay!(resident, "resident", file.behavior.resident);
    overlay!(multi, "multi", file.behavior.multi);
    overlay!(debug, "debug", file.behavior.debug);
    if !was_set_on_cli(matches, "wm")
        && let Some(v) = file.behavior.wm
    {
        cli.wm = Some(v);
    }
    overlay!(hide_timeout, "hide_timeout", file.behavior.hide_timeout);
    overlay!(hotspot_delay, "hotspot_delay", file.behavior.hotspot_delay);
    overlay!(hotspot_layer, "hotspot_layer", file.behavior.hotspot_layer);

    // [layout]
    overlay!(position, "position", file.layout.position);
    overlay!(alignment, "alignment", file.layout.alignment);
    overlay!(full, "full", file.layout.full);
    overlay!(mt, "mt", file.layout.mt);
    overlay!(mb, "mb", file.layout.mb);
    overlay!(ml, "ml", file.layout.ml);
    overlay!(mr, "mr", file.layout.mr);
    overlay!(output, "output", file.layout.output);
    overlay!(layer, "layer", file.layout.layer);
    overlay!(exclusive, "exclusive", file.layout.exclusive);

    // [appearance]
    overlay!(icon_size, "icon_size", file.appearance.icon_size);
    overlay!(opacity, "opacity", file.appearance.opacity);
    overlay!(css_file, "css_file", file.appearance.css_file);
    overlay!(
        launch_animation,
        "launch_animation",
        file.appearance.launch_animation
    );

    // [launcher]
    overlay!(launcher_cmd, "launcher_cmd", file.launcher.launcher_cmd);
    overlay!(launcher_pos, "launcher_pos", file.launcher.launcher_pos);
    overlay!(nolauncher, "nolauncher", file.launcher.nolauncher);
    overlay!(ico, "ico", file.launcher.ico);

    // [filters] — StringOrList collapsed to canonical separator
    if !was_set_on_cli(matches, "ignore_classes")
        && let Some(v) = file.filters.ignore_classes
    {
        cli.ignore_classes = v.into_string(" ");
    }
    if !was_set_on_cli(matches, "ignore_workspaces")
        && let Some(v) = file.filters.ignore_workspaces
    {
        cli.ignore_workspaces = v.into_string(",");
    }
    overlay!(num_ws, "num_ws", file.filters.num_ws);
    overlay!(
        no_fullscreen_suppress,
        "no_fullscreen_suppress",
        file.filters.no_fullscreen_suppress
    );

    cli
}

fn was_set_on_cli(matches: &clap::ArgMatches, id: &str) -> bool {
    matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Position;
    use crate::config_file::schema::{AppearanceSection, StringOrList};
    use nwg_common::compositor::WmOverride;

    use clap::{CommandFactory, FromArgMatches};

    fn parse(args: &[&str]) -> (clap::ArgMatches, DockConfig) {
        let cmd = DockConfig::command();
        let matches = cmd.try_get_matches_from(args).unwrap();
        let cfg = DockConfig::from_arg_matches(&matches).unwrap();
        (matches, cfg)
    }

    fn file_with_icon_size(n: i32) -> RawConfigFile {
        RawConfigFile {
            appearance: AppearanceSection {
                icon_size: Some(n),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn merge_cli_explicit_beats_file() {
        let (matches, cli) = parse(&["test", "--icon-size", "32"]);
        let merged = merge(&matches, cli, Some(file_with_icon_size(64)));
        assert_eq!(merged.icon_size, 32);
    }

    #[test]
    fn merge_file_beats_cli_default() {
        let (matches, cli) = parse(&["test"]);
        let merged = merge(&matches, cli, Some(file_with_icon_size(64)));
        assert_eq!(merged.icon_size, 64);
    }

    #[test]
    fn merge_defaults_when_neither() {
        let (matches, cli) = parse(&["test"]);
        let merged = merge(&matches, cli, None);
        assert_eq!(merged.icon_size, 48);
    }

    #[test]
    fn merge_cli_explicit_default_value_still_wins() {
        // User passes `--icon-size 48` explicitly (which happens to equal
        // the default). value_source must report CommandLine, so the file
        // value (64) does NOT override.
        let (matches, cli) = parse(&["test", "--icon-size", "48"]);
        let merged = merge(&matches, cli, Some(file_with_icon_size(64)));
        assert_eq!(merged.icon_size, 48);
    }

    #[test]
    fn merge_string_field_file_wins_over_default() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.launcher.launcher_cmd = Some("custom-launcher".into());
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.launcher_cmd, "custom-launcher");
    }

    #[test]
    fn merge_enum_field_file_wins_over_default() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.layout.position = Some(Position::Top);
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.position, Position::Top);
    }

    #[test]
    fn merge_bool_flag_file_wins_when_cli_absent() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.behavior.autohide = Some(true);
        let merged = merge(&matches, cli, Some(file));
        assert!(merged.autohide);
    }

    #[test]
    fn merge_string_or_list_array_form_joins_for_classes() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.filters.ignore_classes = Some(StringOrList::List(vec!["a".into(), "b".into()]));
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.ignore_classes, "a b");
    }

    #[test]
    fn merge_string_or_list_array_form_joins_for_workspaces() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.filters.ignore_workspaces = Some(StringOrList::List(vec!["1".into(), "2".into()]));
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.ignore_workspaces, "1,2");
    }

    #[test]
    fn merge_wm_override_field_wins_when_cli_absent() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.behavior.wm = Some(WmOverride::Sway);
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.wm, Some(WmOverride::Sway));
    }
}
