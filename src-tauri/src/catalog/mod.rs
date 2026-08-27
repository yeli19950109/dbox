use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::domain::{
    Component, ComponentId, Installation, PackageKind, ProviderId, Strategy, StrategyId,
    StrategyKind, Tool, ToolId,
};
use crate::version::{ComponentStatus, StatusReason};

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;
pub const PI_BUILTIN_MANIFEST: &str = include_str!("../../resources/catalog/pi.toml");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestV1 {
    pub schema_version: u32,
    pub id: ToolId,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub categories: Option<Vec<String>>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub discovery: Vec<DiscoveryRule>,
    #[serde(default)]
    pub installations: Vec<InstallationMatcher>,
    #[serde(default)]
    pub components: Vec<ComponentManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryRule {
    Executable { names: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationMatcher {
    pub id: String,
    #[serde(alias = "provider")]
    pub provider_id: Option<ProviderId>,
    pub kind: Option<PackageKind>,
    pub package: Option<String>,
    #[serde(default, alias = "executables")]
    pub executable_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionTracking {
    Tracked,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    pub id: ComponentId,
    pub display_name: Option<String>,
    pub default_strategy: Option<StrategyId>,
    pub version_tracking: Option<VersionTracking>,
    pub installed_version: Option<VersionSourceSpec>,
    pub latest_version: Option<VersionSourceSpec>,
    #[serde(default, alias = "strategies")]
    pub update_strategies: Vec<StrategyManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VersionSourceSpec {
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        stream: OutputStream,
        parser: VersionParserSpec,
    },
    NpmRegistry {
        installation: String,
    },
    Provider {
        installation: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VersionParserSpec {
    SemverRegex { pattern: String },
    PlainText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStrategyKind {
    Command,
    PackageManager,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyManifest {
    pub id: StrategyId,
    pub display_name: Option<String>,
    pub kind: Option<ManifestStrategyKind>,
    pub program: Option<String>,
    pub manager: Option<String>,
    pub args: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub env: Option<BTreeMap<String, String>>,
    pub timeout_seconds: Option<u64>,
    pub success_exit_codes: Option<Vec<i32>>,
    pub post_check: Option<VersionSourceSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogLayer {
    BuiltIn,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedManifest {
    pub source: PathBuf,
    pub layer: CatalogLayer,
    pub manifest: ManifestV1,
}

impl LoadedManifest {
    pub fn parse(
        source: impl Into<PathBuf>,
        layer: CatalogLayer,
        contents: &str,
    ) -> Result<Self, CatalogError> {
        let source = source.into();
        let deserializer = toml::Deserializer::parse(contents).map_err(|error| {
            let (line, column) = error
                .span()
                .map(|span| line_and_column(contents, span))
                .unwrap_or((None, None));
            CatalogError {
                source: source.clone(),
                field_path: String::new().into_boxed_str(),
                kind: CatalogErrorKind::Parse,
                message: error.to_string().into_boxed_str(),
                hint: "Fix the TOML syntax shown at this location".into(),
                line,
                column,
            }
        })?;
        let parsed: Result<ManifestV1, _> = serde_path_to_error::deserialize(deserializer);
        let manifest = match parsed {
            Ok(manifest) => manifest,
            Err(error) => {
                let field_path = error.path().to_string();
                let inner = error.into_inner();
                let (line, column) = inner
                    .span()
                    .map(|span| line_and_column(contents, span))
                    .unwrap_or((None, None));
                return Err(CatalogError {
                    source,
                    field_path: field_path.into_boxed_str(),
                    kind: CatalogErrorKind::Parse,
                    message: inner.to_string().into_boxed_str(),
                    hint: "Fix the TOML syntax or field type shown at this location".into(),
                    line,
                    column,
                });
            }
        };

        if manifest.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(CatalogError {
                source,
                field_path: "schema_version".into(),
                kind: CatalogErrorKind::UnsupportedSchemaVersion,
                message: format!(
                    "schema version {} is not supported",
                    manifest.schema_version
                )
                .into_boxed_str(),
                hint: format!("Use schema_version = {SUPPORTED_SCHEMA_VERSION}").into_boxed_str(),
                line: None,
                column: None,
            });
        }

        validate_manifest(&source, &manifest, layer == CatalogLayer::User)?;
        Ok(Self {
            source,
            layer,
            manifest,
        })
    }

    pub fn to_toml(&self) -> Result<String, CatalogError> {
        toml::to_string_pretty(&self.manifest).map_err(|error| CatalogError {
            source: self.source.clone(),
            field_path: String::new().into_boxed_str(),
            kind: CatalogErrorKind::Serialize,
            message: error.to_string().into_boxed_str(),
            hint: "Check the manifest values before serializing".into(),
            line: None,
            column: None,
        })
    }
}

fn line_and_column(contents: &str, span: Range<usize>) -> (Option<usize>, Option<usize>) {
    let offset = span.start.min(contents.len());
    let prefix = &contents[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.chars().count() + 1, |(_, suffix)| {
            suffix.chars().count() + 1
        });
    (Some(line), Some(column))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogErrorKind {
    Io,
    Parse,
    Serialize,
    UnsupportedSchemaVersion,
    DuplicateId,
    InvalidRegex,
    InvalidCommand,
    InvalidMatcher,
    IncompleteDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogError {
    pub source: PathBuf,
    pub field_path: Box<str>,
    pub kind: CatalogErrorKind,
    pub message: Box<str>,
    pub hint: Box<str>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}{}: {}",
            self.source.display(),
            self.field_path,
            location_suffix(&self.line, &self.column),
            self.message
        )
    }
}

impl std::error::Error for CatalogError {}

fn location_suffix(line: &Option<usize>, column: &Option<usize>) -> String {
    match (line, column) {
        (Some(line), Some(column)) => format!(" (line {line}, column {column})"),
        _ => String::new(),
    }
}

pub fn load_manifest_file(
    path: impl AsRef<Path>,
    layer: CatalogLayer,
) -> Result<LoadedManifest, CatalogError> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path).map_err(|error| CatalogError {
        source: path.to_path_buf(),
        field_path: String::new().into_boxed_str(),
        kind: CatalogErrorKind::Io,
        message: error.to_string().into_boxed_str(),
        hint: "Ensure the manifest exists and is readable".into(),
        line: None,
        column: None,
    })?;
    LoadedManifest::parse(path, layer, &contents)
}

pub fn load_manifest_directory(
    path: impl AsRef<Path>,
    layer: CatalogLayer,
) -> Result<Vec<LoadedManifest>, CatalogError> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(path).map_err(|error| CatalogError {
        source: path.to_path_buf(),
        field_path: String::new().into_boxed_str(),
        kind: CatalogErrorKind::Io,
        message: error.to_string().into_boxed_str(),
        hint: "Ensure the catalog directory is readable".into(),
        line: None,
        column: None,
    })?;
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| {
            entry
                .extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|file| load_manifest_file(file, layer))
        .collect()
}

#[derive(Debug, Clone)]
pub struct CatalogLoadReport {
    pub catalog: Catalog,
    pub errors: Vec<CatalogError>,
}

pub fn built_in_manifests() -> Vec<LoadedManifest> {
    vec![LoadedManifest::parse(
        "resources/catalog/pi.toml",
        CatalogLayer::BuiltIn,
        PI_BUILTIN_MANIFEST,
    )
    .expect("the embedded Pi catalog manifest is valid")]
}

pub fn load_catalog_with_built_ins(user_directory: impl AsRef<Path>) -> CatalogLoadReport {
    let built_ins = built_in_manifests();
    let mut accepted_users = Vec::new();
    let mut errors = Vec::new();
    let mut catalog = Catalog::build(built_ins.clone())
        .expect("the embedded catalog manifests form a valid catalog");
    let user_directory = user_directory.as_ref();
    if !user_directory.exists() {
        return CatalogLoadReport { catalog, errors };
    }
    let entries = match fs::read_dir(user_directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(CatalogError {
                source: user_directory.to_path_buf(),
                field_path: String::new().into_boxed_str(),
                kind: CatalogErrorKind::Io,
                message: error.to_string().into_boxed_str(),
                hint: "Ensure the catalog directory is readable".into(),
                line: None,
                column: None,
            });
            return CatalogLoadReport { catalog, errors };
        }
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    files.sort();
    for file in files {
        let loaded = match load_manifest_file(&file, CatalogLayer::User) {
            Ok(loaded) => loaded,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let mut trial = built_ins.clone();
        trial.extend(accepted_users.iter().cloned());
        trial.push(loaded.clone());
        match Catalog::build(trial) {
            Ok(updated) => {
                accepted_users.push(loaded);
                catalog = updated;
            }
            Err(error) => errors.push(error),
        }
    }
    CatalogLoadReport { catalog, errors }
}

fn validate_manifest(
    source: &Path,
    manifest: &ManifestV1,
    allow_patch: bool,
) -> Result<(), CatalogError> {
    validate_unique_ids(
        source,
        "installations",
        manifest
            .installations
            .iter()
            .map(|matcher| matcher.id.as_str()),
    )?;
    validate_unique_ids(
        source,
        "components",
        manifest
            .components
            .iter()
            .map(|component| component.id.as_str()),
    )?;

    for (index, matcher) in manifest.installations.iter().enumerate() {
        if matcher.id.trim().is_empty() {
            return validation_error(
                source,
                format!("installations[{index}].id"),
                CatalogErrorKind::InvalidMatcher,
                "matcher ID cannot be empty",
                "Give each installation matcher a stable ID",
            );
        }
        let constrained = matcher.provider_id.is_some()
            || matcher.kind.is_some()
            || matcher.package.is_some()
            || matcher
                .executable_names
                .as_ref()
                .is_some_and(|names| !names.is_empty());
        if !constrained && !allow_patch {
            return validation_error(
                source,
                format!("installations[{index}]"),
                CatalogErrorKind::InvalidMatcher,
                "matcher has no constraints",
                "Set provider_id, kind, package, or executable_names",
            );
        }
    }

    for (component_index, component) in manifest.components.iter().enumerate() {
        validate_unique_ids(
            source,
            &format!("components[{component_index}].update_strategies"),
            component
                .update_strategies
                .iter()
                .map(|strategy| strategy.id.as_str()),
        )?;
        validate_version_source(
            source,
            &format!("components[{component_index}].installed_version"),
            component.installed_version.as_ref(),
        )?;
        validate_version_source(
            source,
            &format!("components[{component_index}].latest_version"),
            component.latest_version.as_ref(),
        )?;
        for (strategy_index, strategy) in component.update_strategies.iter().enumerate() {
            validate_strategy(
                source,
                &format!("components[{component_index}].update_strategies[{strategy_index}]"),
                strategy,
                allow_patch,
            )?;
        }
    }
    Ok(())
}

fn validate_unique_ids<'a>(
    source: &Path,
    field_path: &str,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), CatalogError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return validation_error(
                source,
                field_path,
                CatalogErrorKind::DuplicateId,
                format!("duplicate child ID `{id}`"),
                "Remove or rename the duplicate item",
            );
        }
    }
    Ok(())
}

fn validate_version_source(
    source: &Path,
    field_path: &str,
    version_source: Option<&VersionSourceSpec>,
) -> Result<(), CatalogError> {
    let Some(VersionSourceSpec::Command {
        program,
        args,
        parser,
        ..
    }) = version_source
    else {
        return Ok(());
    };
    validate_program(source, field_path, program, args)?;
    if let VersionParserSpec::SemverRegex { pattern } = parser {
        Regex::new(pattern).map_err(|error| CatalogError {
            source: source.to_path_buf(),
            field_path: format!("{field_path}.parser.pattern").into_boxed_str(),
            kind: CatalogErrorKind::InvalidRegex,
            message: error.to_string().into_boxed_str(),
            hint: "Use a valid Rust regular expression with a semantic-version capture".into(),
            line: None,
            column: None,
        })?;
    }
    Ok(())
}

fn validate_strategy(
    source: &Path,
    field_path: &str,
    strategy: &StrategyManifest,
    allow_patch: bool,
) -> Result<(), CatalogError> {
    match strategy.kind {
        Some(ManifestStrategyKind::Command) => {
            let program = strategy.program.as_deref().ok_or_else(|| CatalogError {
                source: source.to_path_buf(),
                field_path: format!("{field_path}.program").into_boxed_str(),
                kind: CatalogErrorKind::IncompleteDefinition,
                message: "command strategy is missing program".into(),
                hint: "Set program to an executable name; commands are stored as argv".into(),
                line: None,
                column: None,
            })?;
            if strategy.manager.is_some() {
                return validation_error(
                    source,
                    format!("{field_path}.manager"),
                    CatalogErrorKind::InvalidCommand,
                    "command strategy cannot set manager",
                    "Use program for command strategies",
                );
            }
            validate_program(
                source,
                field_path,
                program,
                strategy.args.as_deref().unwrap_or_default(),
            )?;
        }
        Some(ManifestStrategyKind::PackageManager) => {
            let manager = strategy.manager.as_deref().ok_or_else(|| CatalogError {
                source: source.to_path_buf(),
                field_path: format!("{field_path}.manager").into_boxed_str(),
                kind: CatalogErrorKind::IncompleteDefinition,
                message: "package-manager strategy is missing manager".into(),
                hint: "Set manager to a provider-supported package manager".into(),
                line: None,
                column: None,
            })?;
            if manager.trim().is_empty() || manager.chars().any(char::is_control) {
                return validation_error(
                    source,
                    format!("{field_path}.manager"),
                    CatalogErrorKind::InvalidCommand,
                    "manager cannot be empty or contain control characters",
                    "Use a package-manager executable name such as npm",
                );
            }
            if strategy.program.is_some() {
                return validation_error(
                    source,
                    format!("{field_path}.program"),
                    CatalogErrorKind::InvalidCommand,
                    "package-manager strategy cannot set program",
                    "Use manager for package-manager strategies",
                );
            }
        }
        None if !allow_patch => {
            return validation_error(
                source,
                format!("{field_path}.kind"),
                CatalogErrorKind::IncompleteDefinition,
                "strategy kind is required",
                "Set kind to command or package_manager",
            );
        }
        None => {}
    }
    if strategy.timeout_seconds == Some(0) {
        return validation_error(
            source,
            format!("{field_path}.timeout_seconds"),
            CatalogErrorKind::InvalidCommand,
            "timeout must be greater than zero",
            "Use a positive timeout in seconds",
        );
    }
    validate_version_source(
        source,
        &format!("{field_path}.post_check"),
        strategy.post_check.as_ref(),
    )
}

fn validate_program(
    source: &Path,
    field_path: &str,
    program: &str,
    args: &[String],
) -> Result<(), CatalogError> {
    if program.trim().is_empty()
        || program.chars().any(char::is_control)
        || args
            .iter()
            .any(|arg| arg.chars().any(|character| character == '\0'))
    {
        return validation_error(
            source,
            format!("{field_path}.program"),
            CatalogErrorKind::InvalidCommand,
            "program and arguments must be non-empty argv values without NUL",
            "Store a program and each argument separately",
        );
    }
    let basename = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    if matches!(basename, "sh" | "bash" | "zsh") && args.iter().any(|arg| arg == "-c") {
        return validation_error(
            source,
            field_path,
            CatalogErrorKind::InvalidCommand,
            "shell command strings are not supported",
            "Use the target program and an explicit args array",
        );
    }
    Ok(())
}

fn validation_error<T>(
    source: &Path,
    field_path: impl Into<Box<str>>,
    kind: CatalogErrorKind,
    message: impl Into<Box<str>>,
    hint: impl Into<Box<str>>,
) -> Result<T, CatalogError> {
    Err(CatalogError {
        source: source.to_path_buf(),
        field_path: field_path.into(),
        kind,
        message: message.into(),
        hint: hint.into(),
        line: None,
        column: None,
    })
}

#[derive(Debug, Clone)]
struct CatalogEntry {
    source: PathBuf,
    manifest: ManifestV1,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: BTreeMap<ToolId, CatalogEntry>,
}

impl Catalog {
    pub fn build(
        manifests: impl IntoIterator<Item = LoadedManifest>,
    ) -> Result<Self, CatalogError> {
        let mut built_ins = BTreeMap::<ToolId, CatalogEntry>::new();
        let mut users = BTreeMap::<ToolId, CatalogEntry>::new();
        for loaded in manifests {
            let target = match loaded.layer {
                CatalogLayer::BuiltIn => &mut built_ins,
                CatalogLayer::User => &mut users,
            };
            let id = loaded.manifest.id.clone();
            if target.contains_key(&id) {
                return validation_error(
                    &loaded.source,
                    "id",
                    CatalogErrorKind::DuplicateId,
                    format!("duplicate manifest ID `{id}` in the same catalog layer"),
                    "Keep one manifest per tool ID in each layer",
                );
            }
            target.insert(
                id,
                CatalogEntry {
                    source: loaded.source,
                    manifest: loaded.manifest,
                },
            );
        }

        for (id, overlay) in users {
            if let Some(base) = built_ins.get_mut(&id) {
                merge_manifest(&mut base.manifest, overlay.manifest);
                base.source = overlay.source;
                validate_manifest(&base.source, &base.manifest, false)?;
            } else {
                validate_manifest(&overlay.source, &overlay.manifest, false)?;
                built_ins.insert(id, overlay);
            }
        }
        for entry in built_ins.values() {
            validate_manifest(&entry.source, &entry.manifest, false)?;
        }
        Ok(Self { entries: built_ins })
    }

    pub fn manifests(&self) -> impl Iterator<Item = (&Path, &ManifestV1)> {
        self.entries
            .values()
            .map(|entry| (entry.source.as_path(), &entry.manifest))
    }

    pub fn enrich(&self, installations: &[Installation]) -> CatalogSnapshot {
        let mut tools: BTreeMap<ToolId, Tool> = installations
            .iter()
            .map(|installation| {
                let tool = Tool::from_installation(installation)
                    .expect("a validated InstallationId is also a valid ToolId");
                (tool.id.clone(), tool)
            })
            .collect();
        let installations_by_id: BTreeMap<_, _> = installations
            .iter()
            .map(|installation| (installation.id.clone(), installation))
            .collect();
        let mut diagnostics = Vec::new();

        for entry in self.entries.values() {
            let matching_tools: BTreeSet<_> = tools
                .values()
                .filter(|tool| tool_matches_manifest(tool, &installations_by_id, &entry.manifest))
                .map(|tool| tool.id.clone())
                .collect();
            match matching_tools.len() {
                0 => diagnostics.push(CatalogDiagnostic {
                    source: entry.source.clone(),
                    manifest_id: entry.manifest.id.clone(),
                    kind: CatalogDiagnosticKind::NoMatch,
                    matching_tool_ids: Vec::new(),
                    message:
                        "manifest did not match an installed tool; automatic tools were retained"
                            .into(),
                }),
                1 => {
                    let tool_id = matching_tools
                        .into_iter()
                        .next()
                        .expect("one matching tool exists");
                    let tool = tools.get_mut(&tool_id).expect("matched tool exists");
                    apply_manifest(tool, &entry.manifest);
                }
                _ => diagnostics.push(CatalogDiagnostic {
                    source: entry.source.clone(),
                    manifest_id: entry.manifest.id.clone(),
                    kind: CatalogDiagnosticKind::AmbiguousMatch,
                    matching_tool_ids: matching_tools.into_iter().collect(),
                    message: "manifest matched multiple tools and was not applied".into(),
                }),
            }
        }

        CatalogSnapshot {
            tools: tools.into_values().collect(),
            diagnostics,
        }
    }
}

fn tool_matches_manifest(
    tool: &Tool,
    installations: &BTreeMap<crate::domain::InstallationId, &Installation>,
    manifest: &ManifestV1,
) -> bool {
    tool.installation_ids.iter().any(|installation_id| {
        let Some(installation) = installations.get(installation_id) else {
            return false;
        };
        manifest
            .installations
            .iter()
            .any(|matcher| matcher_matches_installation(matcher, installation))
            || manifest.discovery.iter().any(|rule| match rule {
                DiscoveryRule::Executable { names } => installation
                    .executables
                    .iter()
                    .any(|executable| names.contains(&executable.name)),
            })
    })
}

fn matcher_matches_installation(
    matcher: &InstallationMatcher,
    installation: &Installation,
) -> bool {
    matcher
        .provider_id
        .as_ref()
        .is_none_or(|provider_id| provider_id == &installation.provider_id)
        && matcher
            .kind
            .as_ref()
            .is_none_or(|kind| kind == &installation.package.kind)
        && matcher
            .package
            .as_ref()
            .is_none_or(|package| package == &installation.package.name)
        && matcher.executable_names.as_ref().is_none_or(|names| {
            installation
                .executables
                .iter()
                .any(|executable| names.contains(&executable.name))
        })
}

fn apply_manifest(tool: &mut Tool, manifest: &ManifestV1) {
    if let Some(display_name) = &manifest.display_name {
        tool.display_name.clone_from(display_name);
    }
    if let Some(categories) = &manifest.categories {
        tool.categories.clone_from(categories);
    }
    if let Some(homepage) = &manifest.homepage {
        tool.homepage = Some(homepage.clone());
    }
    for manifest_component in &manifest.components {
        if let Some(component) = tool
            .components
            .iter_mut()
            .find(|component| component.id == manifest_component.id)
        {
            apply_component_manifest(component, manifest_component);
        } else {
            tool.components
                .push(component_from_manifest(manifest_component));
        }
    }
    tool.components
        .sort_by(|left, right| left.id.cmp(&right.id));
}

fn component_from_manifest(manifest: &ComponentManifest) -> Component {
    let mut component = Component {
        id: manifest.id.clone(),
        display_name: manifest
            .display_name
            .clone()
            .unwrap_or_else(|| manifest.id.to_string()),
        installation_id: None,
        strategies: Vec::new(),
        default_strategy: manifest.default_strategy.clone(),
        installed_version: None,
        latest_version: None,
        status: ComponentStatus::unknown("not_checked", "Updates have not been checked"),
    };
    apply_component_manifest(&mut component, manifest);
    component
}

fn apply_component_manifest(component: &mut Component, manifest: &ComponentManifest) {
    if let Some(display_name) = &manifest.display_name {
        component.display_name.clone_from(display_name);
    }
    if manifest.default_strategy.is_some() {
        component
            .default_strategy
            .clone_from(&manifest.default_strategy);
    }
    if manifest.version_tracking == Some(VersionTracking::Unsupported) {
        component.status = ComponentStatus::Unsupported {
            reason: StatusReason::new(
                "version_tracking_unsupported",
                "This component has no reliable version source",
                false,
            ),
        };
    }
    for manifest_strategy in &manifest.update_strategies {
        let strategy = strategy_from_manifest(manifest_strategy);
        if let Some(existing) = component
            .strategies
            .iter_mut()
            .find(|existing| existing.id == strategy.id)
        {
            *existing = strategy;
        } else {
            component.strategies.push(strategy);
        }
    }
    component
        .strategies
        .sort_by(|left, right| left.id.cmp(&right.id));
}

fn strategy_from_manifest(manifest: &StrategyManifest) -> Strategy {
    let args = manifest.args.clone().unwrap_or_default();
    let kind = match manifest
        .kind
        .expect("merged catalog strategies have a validated kind")
    {
        ManifestStrategyKind::Command => StrategyKind::Command {
            program: manifest
                .program
                .clone()
                .expect("validated command strategy has a program"),
            args,
        },
        ManifestStrategyKind::PackageManager => StrategyKind::PackageManager {
            manager: manifest
                .manager
                .clone()
                .expect("validated package-manager strategy has a manager"),
            args,
        },
    };
    Strategy {
        id: manifest.id.clone(),
        display_name: manifest
            .display_name
            .clone()
            .unwrap_or_else(|| manifest.id.to_string()),
        kind,
        enabled: true,
    }
}

fn merge_manifest(base: &mut ManifestV1, overlay: ManifestV1) {
    overwrite_option(&mut base.display_name, overlay.display_name);
    overwrite_option(&mut base.description, overlay.description);
    overwrite_option(&mut base.categories, overlay.categories);
    overwrite_option(&mut base.homepage, overlay.homepage);
    if !overlay.discovery.is_empty() {
        base.discovery = overlay.discovery;
    }
    merge_installation_matchers(&mut base.installations, overlay.installations);
    merge_components(&mut base.components, overlay.components);
}

fn overwrite_option<T>(base: &mut Option<T>, overlay: Option<T>) {
    if overlay.is_some() {
        *base = overlay;
    }
}

fn merge_installation_matchers(
    base: &mut Vec<InstallationMatcher>,
    overlay: Vec<InstallationMatcher>,
) {
    let mut merged: BTreeMap<_, _> = base.drain(..).map(|item| (item.id.clone(), item)).collect();
    for patch in overlay {
        if let Some(item) = merged.get_mut(&patch.id) {
            overwrite_option(&mut item.provider_id, patch.provider_id);
            overwrite_option(&mut item.kind, patch.kind);
            overwrite_option(&mut item.package, patch.package);
            overwrite_option(&mut item.executable_names, patch.executable_names);
        } else {
            merged.insert(patch.id.clone(), patch);
        }
    }
    *base = merged.into_values().collect();
}

fn merge_components(base: &mut Vec<ComponentManifest>, overlay: Vec<ComponentManifest>) {
    let mut merged: BTreeMap<_, _> = base.drain(..).map(|item| (item.id.clone(), item)).collect();
    for mut patch in overlay {
        if let Some(item) = merged.get_mut(&patch.id) {
            overwrite_option(&mut item.display_name, patch.display_name);
            overwrite_option(&mut item.default_strategy, patch.default_strategy);
            overwrite_option(&mut item.version_tracking, patch.version_tracking);
            overwrite_option(&mut item.installed_version, patch.installed_version);
            overwrite_option(&mut item.latest_version, patch.latest_version);
            merge_strategies(&mut item.update_strategies, patch.update_strategies);
        } else {
            patch
                .update_strategies
                .sort_by(|left, right| left.id.cmp(&right.id));
            merged.insert(patch.id.clone(), patch);
        }
    }
    *base = merged.into_values().collect();
}

fn merge_strategies(base: &mut Vec<StrategyManifest>, overlay: Vec<StrategyManifest>) {
    let mut merged: BTreeMap<_, _> = base.drain(..).map(|item| (item.id.clone(), item)).collect();
    for patch in overlay {
        if let Some(item) = merged.get_mut(&patch.id) {
            overwrite_option(&mut item.display_name, patch.display_name);
            overwrite_option(&mut item.kind, patch.kind);
            overwrite_option(&mut item.program, patch.program);
            overwrite_option(&mut item.manager, patch.manager);
            overwrite_option(&mut item.args, patch.args);
            overwrite_option(&mut item.cwd, patch.cwd);
            overwrite_option(&mut item.env, patch.env);
            overwrite_option(&mut item.timeout_seconds, patch.timeout_seconds);
            overwrite_option(&mut item.success_exit_codes, patch.success_exit_codes);
            overwrite_option(&mut item.post_check, patch.post_check);
        } else {
            merged.insert(patch.id.clone(), patch);
        }
    }
    *base = merged.into_values().collect();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogDiagnosticKind {
    NoMatch,
    AmbiguousMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogDiagnostic {
    pub source: PathBuf,
    pub manifest_id: ToolId,
    pub kind: CatalogDiagnosticKind,
    pub matching_tool_ids: Vec<ToolId>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshot {
    pub tools: Vec<Tool>,
    pub diagnostics: Vec<CatalogDiagnostic>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Executable, InstallationScope, PackageCoordinate};

    const PI_MANIFEST: &str = r#"
schema_version = 1
id = "pi"
display_name = "Pi"
categories = ["agent", "tui"]
homepage = "https://pi.dev"

[[discovery]]
kind = "executable"
names = ["pi"]

[[installations]]
id = "npm"
kind = "npm_global"
package = "@earendil-works/pi-coding-agent"

[[components]]
id = "core"
display_name = "Core"
default_strategy = "self"

[components.installed_version]
kind = "command"
program = "pi"
args = ["--version"]
stream = "stdout"
parser = { kind = "semver_regex", pattern = "([0-9]+\\.[0-9]+\\.[0-9]+)" }

[components.latest_version]
kind = "npm_registry"
installation = "npm"

[[components.update_strategies]]
id = "self"
display_name = "Pi self update"
kind = "command"
program = "pi"
args = ["update", "--self"]
timeout_seconds = 600

[[components.update_strategies]]
id = "npm-global"
display_name = "npm global"
kind = "package_manager"
manager = "npm"
args = ["install", "--global", "--ignore-scripts", "@earendil-works/pi-coding-agent@latest"]
timeout_seconds = 600

[[components]]
id = "extensions"
display_name = "Extensions"
default_strategy = "pi-extensions"
version_tracking = "unsupported"

[[components.update_strategies]]
id = "pi-extensions"
display_name = "Pi extensions update"
kind = "command"
program = "pi"
args = ["update", "--extensions"]
timeout_seconds = 900
"#;

    fn parse(source: &str, layer: CatalogLayer, contents: &str) -> LoadedManifest {
        LoadedManifest::parse(source, layer, contents).unwrap()
    }

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
            path: PathBuf::from(format!("/path with spaces/{executable}")),
        }]);
        installation
    }

    #[test]
    fn documented_pi_manifest_parses_and_round_trips() {
        let loaded = parse("builtin/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST);
        let encoded = loaded.to_toml().unwrap();
        let decoded = parse("roundtrip.toml", CatalogLayer::BuiltIn, &encoded);
        assert_eq!(loaded.manifest, decoded.manifest);
    }

    #[test]
    fn automatic_tool_remains_manageable_without_a_manifest() {
        let installation = installation(
            "npm-global",
            PackageKind::NpmGlobal,
            "unknown-package",
            "unknown-cli",
        );
        let snapshot = Catalog::default().enrich(std::slice::from_ref(&installation));
        assert_eq!(snapshot.tools.len(), 1);
        assert_eq!(snapshot.tools[0].installation_ids, vec![installation.id]);
        assert_eq!(snapshot.tools[0].components[0].strategies.len(), 1);
    }

    #[test]
    fn removing_an_enrichment_never_removes_the_installation() {
        let installation = installation(
            "npm-global",
            PackageKind::NpmGlobal,
            "@earendil-works/pi-coding-agent",
            "pi",
        );
        let catalog =
            Catalog::build([parse("builtin/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST)]).unwrap();
        assert_eq!(
            catalog.enrich(std::slice::from_ref(&installation)).tools[0]
                .components
                .len(),
            2
        );
        assert_eq!(Catalog::default().enrich(&[installation]).tools.len(), 1);
    }

    #[test]
    fn user_can_override_one_strategy_without_copying_the_builtin_definition() {
        let user_patch = r#"
schema_version = 1
id = "pi"

[[components]]
id = "core"

[[components.update_strategies]]
id = "self"
display_name = "Preferred self update"
timeout_seconds = 30
"#;
        let catalog = Catalog::build([
            parse("builtin/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST),
            parse("tools.d/pi.toml", CatalogLayer::User, user_patch),
        ])
        .unwrap();
        let manifest = catalog.manifests().next().unwrap().1;
        let strategy = manifest
            .components
            .iter()
            .find(|component| component.id.as_str() == "core")
            .unwrap()
            .update_strategies
            .iter()
            .find(|strategy| strategy.id.as_str() == "self")
            .unwrap();
        assert_eq!(
            strategy.display_name.as_deref(),
            Some("Preferred self update")
        );
        assert_eq!(strategy.program.as_deref(), Some("pi"));
        assert_eq!(strategy.args.as_ref().unwrap(), &["update", "--self"]);
        assert_eq!(strategy.timeout_seconds, Some(30));
    }

    #[test]
    fn multiple_matchers_for_one_tool_apply_once() {
        let installation = installation(
            "npm-global",
            PackageKind::NpmGlobal,
            "@earendil-works/pi-coding-agent",
            "pi",
        );
        let catalog =
            Catalog::build([parse("builtin/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST)]).unwrap();
        let snapshot = catalog.enrich(&[installation]);
        assert!(snapshot.diagnostics.is_empty());
        assert_eq!(snapshot.tools.len(), 1);
        assert_eq!(snapshot.tools[0].display_name, "Pi");
    }

    #[test]
    fn zero_match_is_reported_without_removing_automatic_tools() {
        let installation = installation("homebrew", PackageKind::HomebrewFormula, "jq", "jq");
        let catalog =
            Catalog::build([parse("builtin/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST)]).unwrap();
        let snapshot = catalog.enrich(&[installation]);
        assert_eq!(snapshot.tools.len(), 1);
        assert_eq!(snapshot.diagnostics[0].kind, CatalogDiagnosticKind::NoMatch);
    }

    #[test]
    fn ambiguous_executable_match_is_reported_and_not_applied() {
        let npm = installation("npm-global", PackageKind::NpmGlobal, "different-pi", "pi");
        let brew = installation(
            "homebrew",
            PackageKind::HomebrewFormula,
            "different-pi",
            "pi",
        );
        let catalog =
            Catalog::build([parse("builtin/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST)]).unwrap();
        let snapshot = catalog.enrich(&[npm, brew]);
        assert_eq!(snapshot.tools.len(), 2);
        assert_eq!(
            snapshot.diagnostics[0].kind,
            CatalogDiagnosticKind::AmbiguousMatch
        );
        assert!(snapshot.tools.iter().all(|tool| tool.display_name != "Pi"));
    }

    #[test]
    fn rejects_future_schema_unknown_fields_duplicates_regex_and_shell_commands() {
        let future = PI_MANIFEST.replacen("schema_version = 1", "schema_version = 99", 1);
        assert_eq!(
            LoadedManifest::parse("future.toml", CatalogLayer::BuiltIn, &future)
                .unwrap_err()
                .kind,
            CatalogErrorKind::UnsupportedSchemaVersion
        );

        let unknown = format!("{PI_MANIFEST}\nunknown_field = true\n");
        let error =
            LoadedManifest::parse("unknown.toml", CatalogLayer::BuiltIn, &unknown).unwrap_err();
        assert_eq!(error.kind, CatalogErrorKind::Parse);
        assert_eq!(error.source, PathBuf::from("unknown.toml"));

        let duplicate = PI_MANIFEST.replace(
            "[[components]]\nid = \"extensions\"",
            "[[components]]\nid = \"core\"",
        );
        assert_eq!(
            LoadedManifest::parse("duplicate.toml", CatalogLayer::BuiltIn, &duplicate)
                .unwrap_err()
                .kind,
            CatalogErrorKind::DuplicateId
        );

        let bad_regex = PI_MANIFEST.replace("([0-9]+\\\\.[0-9]+\\\\.[0-9]+)", "([unclosed");
        assert_eq!(
            LoadedManifest::parse("regex.toml", CatalogLayer::BuiltIn, &bad_regex)
                .unwrap_err()
                .kind,
            CatalogErrorKind::InvalidRegex
        );

        let shell = PI_MANIFEST.replace(
            "program = \"pi\"\nargs = [\"update\", \"--self\"]",
            "program = \"sh\"\nargs = [\"-c\", \"pi update --self\"]",
        );
        assert_eq!(
            LoadedManifest::parse("shell.toml", CatalogLayer::BuiltIn, &shell)
                .unwrap_err()
                .kind,
            CatalogErrorKind::InvalidCommand
        );
    }

    #[test]
    fn duplicate_manifest_ids_in_one_layer_are_rejected() {
        let first = parse("a/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST);
        let second = parse("b/pi.toml", CatalogLayer::BuiltIn, PI_MANIFEST);
        let error = Catalog::build([first, second]).unwrap_err();
        assert_eq!(error.kind, CatalogErrorKind::DuplicateId);
        assert_eq!(error.source, PathBuf::from("b/pi.toml"));
    }
}
