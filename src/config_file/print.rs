use super::schema::{
    AppearanceSection, BehaviorSection, FiltersSection, LauncherSection, LayoutSection,
    RawConfigFile, StringOrList,
};
use crate::config::DockConfig;

/// Serializes a fully-resolved `DockConfig` to a TOML string with the
/// same five-section schema the file uses. Used by `--print-config`.
///
/// Every field is emitted with its current value so the output is a
/// "what the dock thinks right now" snapshot.
pub(crate) fn print_effective_config(cfg: &DockConfig) -> String {
    let raw = RawConfigFile {
        behavior: BehaviorSection {
            autohide: Some(cfg.autohide),
            resident: Some(cfg.resident),
            multi: Some(cfg.multi),
            debug: Some(cfg.debug),
            wm: cfg.wm,
            hide_timeout: Some(cfg.hide_timeout),
            hotspot_delay: Some(cfg.hotspot_delay),
            hotspot_layer: Some(cfg.hotspot_layer),
        },
        layout: LayoutSection {
            position: Some(cfg.position),
            alignment: Some(cfg.alignment),
            full: Some(cfg.full),
            mt: Some(cfg.mt),
            mb: Some(cfg.mb),
            ml: Some(cfg.ml),
            mr: Some(cfg.mr),
            output: Some(cfg.output.clone()),
            layer: Some(cfg.layer),
            exclusive: Some(cfg.exclusive),
        },
        appearance: AppearanceSection {
            icon_size: Some(cfg.icon_size),
            opacity: Some(cfg.opacity),
            css_file: Some(cfg.css_file.clone()),
            launch_animation: Some(cfg.launch_animation),
        },
        launcher: LauncherSection {
            launcher_cmd: Some(cfg.launcher_cmd.clone()),
            launcher_pos: Some(cfg.launcher_pos),
            nolauncher: Some(cfg.nolauncher),
            ico: Some(cfg.ico.clone()),
        },
        filters: FiltersSection {
            ignore_classes: Some(StringOrList::String(cfg.ignore_classes.clone())),
            ignore_workspaces: Some(StringOrList::String(cfg.ignore_workspaces.clone())),
            num_ws: Some(cfg.num_ws),
            no_fullscreen_suppress: Some(cfg.no_fullscreen_suppress),
        },
    };
    toml::to_string_pretty(&raw).unwrap_or_else(|e| {
        // Serializing should never fail for our well-typed schema, but
        // returning a usable string keeps --print-config from panicking.
        format!("# print_effective_config serialization failed: {e}\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Position;
    use clap::{CommandFactory, FromArgMatches};

    fn parse(args: &[&str]) -> (clap::ArgMatches, DockConfig) {
        let cmd = DockConfig::command();
        let matches = cmd.try_get_matches_from(args).unwrap();
        let cfg = DockConfig::from_arg_matches(&matches).unwrap();
        (matches, cfg)
    }

    // ─── print_effective_config round-trip ─────────────────────────────────

    #[test]
    fn print_then_parse_round_trip_yields_same_values() {
        let (_, mut cli) = parse(&["test"]);
        cli.icon_size = 64;
        cli.position = Position::Left;
        cli.opacity = 75;

        let s = print_effective_config(&cli);
        let raw: RawConfigFile = toml::from_str(&s).unwrap();

        assert_eq!(raw.appearance.icon_size, Some(64));
        assert_eq!(raw.layout.position, Some(Position::Left));
        assert_eq!(raw.appearance.opacity, Some(75));
    }

    #[test]
    fn print_emits_all_five_sections() {
        let (_, cli) = parse(&["test"]);
        let s = print_effective_config(&cli);
        for header in [
            "[behavior]",
            "[layout]",
            "[appearance]",
            "[launcher]",
            "[filters]",
        ] {
            assert!(s.contains(header), "expected {header} in:\n{s}");
        }
    }
}
