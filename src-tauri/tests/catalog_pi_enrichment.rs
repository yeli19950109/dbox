use std::collections::BTreeMap;
use std::path::PathBuf;

use dbox_lib::catalog::{
    built_in_manifests, load_catalog_with_built_ins, Catalog, CatalogDiagnosticKind,
};
use dbox_lib::domain::{
    ComponentId, Executable, Installation, InstallationScope, PackageCoordinate, PackageKind,
    ProviderId, StrategyId, StrategyKind,
};
use dbox_lib::persistence::{AppPaths, PersistenceStore};
use tempfile::TempDir;

fn installation(
    provider: &str,
    kind: PackageKind,
    package: &str,
    executable: &str,
) -> Installation {
    let mut installation = Installation::new(
        ProviderId::new(provider).unwrap(),
        PackageCoordinate::new(kind, package).unwrap(),
        InstallationScope::Global,
    )
    .unwrap();
    installation.set_executables([Executable {
        name: executable.into(),
        path: PathBuf::from(format!("/fixture/bin/{executable}")),
    }]);
    installation
}

#[test]
fn built_in_pi_enriches_one_generic_installation_without_hiding_unknown_tools() {
    let pi = installation(
        "npm-global",
        PackageKind::NpmGlobal,
        "@earendil-works/pi-coding-agent",
        "pi",
    );
    let unknown_npm = installation(
        "npm-global",
        PackageKind::NpmGlobal,
        "random-npm-cli",
        "random-npm",
    );
    let unknown_brew = installation(
        "homebrew",
        PackageKind::HomebrewFormula,
        "random-brew-cli",
        "random-brew",
    );
    let catalog = Catalog::build(built_in_manifests()).unwrap();
    let snapshot = catalog.enrich(&[pi.clone(), unknown_npm.clone(), unknown_brew.clone()]);

    assert_eq!(snapshot.tools.len(), 3);
    let pi_tools: Vec<_> = snapshot
        .tools
        .iter()
        .filter(|tool| tool.display_name == "Pi")
        .collect();
    assert_eq!(pi_tools.len(), 1, "Pi is enhanced rather than duplicated");
    assert_eq!(pi_tools[0].installation_ids, [pi.id]);
    assert!(snapshot
        .tools
        .iter()
        .any(|tool| tool.installation_ids == [unknown_npm.id.clone()]));
    assert!(snapshot
        .tools
        .iter()
        .any(|tool| tool.installation_ids == [unknown_brew.id.clone()]));

    let core = pi_tools[0]
        .components
        .iter()
        .find(|component| component.id.as_str() == "core")
        .unwrap();
    assert_eq!(core.default_strategy.as_ref().unwrap().as_str(), "self");
    assert!(core
        .strategies
        .iter()
        .any(|strategy| strategy.id.as_str() == "self"));
    assert!(core
        .strategies
        .iter()
        .any(|strategy| strategy.id.as_str() == "npm-global"));

    // Extensions are independent installations from the pi-extensions provider.
    // The core must not retain an unsupported placeholder that hides its real status.
    assert_eq!(pi_tools[0].components.len(), 1);
    assert!(pi_tools[0]
        .components
        .iter()
        .all(|component| component
            .strategies
            .iter()
            .all(|strategy| match &strategy.kind {
                StrategyKind::Command { args, .. } | StrategyKind::PackageManager { args, .. } =>
                    !args.iter().any(|argument| argument == "--all"),
                StrategyKind::ProviderDefault => true,
            })));
}

#[test]
fn mise_pi_backends_keep_their_own_update_strategy_alongside_npm_pi() {
    let catalog = Catalog::build(built_in_manifests()).unwrap();
    for backend in [
        "aqua:earendil-works/pi",
        "github:earendil-works/pi",
        "npm:@earendil-works/pi-coding-agent",
    ] {
        let mut mise_pi = installation("mise", PackageKind::Mise, backend, "pi");
        // Mise discovery reports installation slots, not PATH executables.
        mise_pi.executables.clear();
        let npm_pi = installation(
            "npm-global",
            PackageKind::NpmGlobal,
            "@earendil-works/pi-coding-agent",
            "pi",
        );
        let snapshot = catalog.enrich(&[mise_pi.clone(), npm_pi.clone()]);
        assert_eq!(snapshot.tools.len(), 2);
        assert!(snapshot.tools.iter().all(|tool| tool.display_name == "Pi"));
        let core = &snapshot
            .tools
            .iter()
            .find(|tool| tool.installation_ids == [mise_pi.id.clone()])
            .unwrap()
            .components[0];
        assert_eq!(
            core.default_strategy.as_ref().unwrap().as_str(),
            "provider-default"
        );
        assert_eq!(core.strategies.len(), 1);
        assert_eq!(core.strategies[0].kind, StrategyKind::ProviderDefault);
        let npm_core = &snapshot
            .tools
            .iter()
            .find(|tool| tool.installation_ids == [npm_pi.id.clone()])
            .unwrap()
            .components[0];
        assert_eq!(npm_core.default_strategy.as_ref().unwrap().as_str(), "self");
    }
}

#[test]
fn a_non_matching_pi_named_package_remains_a_normal_automatic_tool() {
    let plain = installation(
        "npm-global",
        PackageKind::NpmGlobal,
        "pi",
        "different-command",
    );
    let catalog = Catalog::build(built_in_manifests()).unwrap();
    let snapshot = catalog.enrich(std::slice::from_ref(&plain));
    assert_eq!(snapshot.tools.len(), 1);
    assert_eq!(snapshot.tools[0].installation_ids, [plain.id]);
    assert_eq!(snapshot.tools[0].components.len(), 1);
    assert_eq!(snapshot.diagnostics[0].kind, CatalogDiagnosticKind::NoMatch);
}

#[test]
fn invalid_user_catalog_only_reports_its_own_error() {
    let temporary = TempDir::new().unwrap();
    std::fs::write(
        temporary.path().join("broken.toml"),
        "schema_version = 1\nid = [not valid",
    )
    .unwrap();
    let report = load_catalog_with_built_ins(temporary.path());
    assert_eq!(report.errors.len(), 1);

    let unknown = installation(
        "homebrew",
        PackageKind::HomebrewFormula,
        "still-visible",
        "still-visible",
    );
    let snapshot = report.catalog.enrich(std::slice::from_ref(&unknown));
    assert_eq!(snapshot.tools.len(), 1);
    assert_eq!(snapshot.tools[0].installation_ids, [unknown.id]);
}

#[test]
fn selected_core_strategy_round_trips_through_settings() {
    let temporary = TempDir::new().unwrap();
    let store = PersistenceStore::new(AppPaths::new(
        temporary.path().join("config"),
        temporary.path().join("data"),
        temporary.path().join("logs"),
    ));
    let loaded = store.load_settings().unwrap();
    let mut settings = loaded.value;
    settings.component_strategies = BTreeMap::from([(
        dbox_lib::domain::ToolId::new("npm-global:pi").unwrap(),
        BTreeMap::from([(
            ComponentId::new("core").unwrap(),
            StrategyId::new("npm-global").unwrap(),
        )]),
    )]);
    store.save_settings(&loaded.revision, &settings).unwrap();

    let reloaded = store.load_settings().unwrap();
    assert_eq!(
        reloaded.value.component_strategies
            [&dbox_lib::domain::ToolId::new("npm-global:pi").unwrap()]
            [&ComponentId::new("core").unwrap()]
            .as_str(),
        "npm-global"
    );
}
