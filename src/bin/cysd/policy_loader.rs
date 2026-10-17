//! Pure frozen declaration catalog; caller owns file IO, epoch publication and origin authority.
#![allow(dead_code)]

use crate::receiver::{ProofClass, ReceiverPolicy, Recipe, RegistrationOrigin, WeakRisk};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationSource {
    Disk,
    Embedded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    MissingDiskDocument,
    MissingAdapter,
    MissingDeclaration { adapter: String },
    MissingEmbeddedBuiltin { adapter: String },
    MissingEffectiveCommand { adapter: String },
}

/// Diagnostic only. A changed command's predeclared weak class never depends on discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclarationNotice {
    ChangedBuiltinCommand {
        /// What the old basename-only classifier would have said; this is NOT the chosen class.
        legacy_class: ProofClass,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeRecipe {
    Supported(Recipe),
    /// The declaration remains in the catalog with its fixed class. This is not a guessed bin.
    Unsupported(RecipeGap),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeGap {
    UnclosedQuote,
    ShellSyntax,
    PlatformQuoting,
    MissingExecutable,
    AssignmentProvenance,
    LiteralPosixBackslash,
    UnsupportedInterpreter,
    MissingScript,
}

/// Recipe is optional in the native sense, never in the declaration/classification sense.
#[derive(Clone)]
pub struct FrozenPolicy {
    pub policy_epoch: u64,
    pub declaration: String,
    pub config_digest: String,
    pub adapter: String,
    pub recipe: NativeRecipe,
    pub origin: RegistrationOrigin,
    pub class: ProofClass,
    pub weak_risk: Option<WeakRisk>,
}

impl FrozenPolicy {
    pub fn public_summary(&self) -> crate::receiver::ReceiverPolicySummary<'_> {
        crate::receiver::ReceiverPolicySummary {
            policy_epoch: self.policy_epoch,
            adapter: &self.adapter,
            config_digest: &self.config_digest,
            origin: self.origin,
            class: self.class,
            weak_risk: self.weak_risk,
        }
    }

    /// Bridge only when the existing native matcher has a concrete recipe. For a predeclared
    /// weak policy, None requires the explicit weak-witness resolver; do not drop its request.
    /// For FullRequired, None is unresolved native support and MUST NOT select a weak fallback.
    pub fn native_policy(&self) -> Option<ReceiverPolicy> {
        let NativeRecipe::Supported(recipe) = &self.recipe else { return None };
        Some(ReceiverPolicy {
            policy_epoch: self.policy_epoch,
            declaration: self.declaration.clone(),
            config_digest: self.config_digest.clone(),
            adapter: self.adapter.clone(),
            recipe: recipe.clone(),
            origin: self.origin,
            class: self.class,
            weak_risk: self.weak_risk,
        })
    }
}

#[derive(Clone)]
pub struct FrozenContract {
    pub policy: FrozenPolicy,
    pub declaration_source: DeclarationSource,
    /// Unexpanded strings. Never resolve env, $HOME or account paths from this pure loader.
    pub launch_env_pairs: Vec<(String, String)>,
    /// CLI load_agent_spec value, with exactly its six UI/hook layers and stale promotion.
    pub effective_launch_spec: Value,
    pub launch_prompt_markers: Vec<String>,
    pub launch_composer_placeholder: Option<String>,
    /// Separate legacy daemon merger semantics; never label these as CLI effective fields.
    pub receiver_prompt_markers: Option<Vec<String>>,
    pub receiver_composer_placeholder: Option<String>,
    /// Exact JSON VALUES used by the loader (not original file formatting). Keep these private
    /// in status/receipts: the source records may contain account-specific values.
    pub disk_record: Option<Value>,
    pub embedded_record: Option<Value>,
    pub notices: Vec<DeclarationNotice>,
}

/// Deliberately no Debug/Serialize on the catalog or contract: raw declaration/env may be private.
pub struct Catalog {
    entries: BTreeMap<String, FrozenContract>,
    rejected: BTreeMap<String, LoadError>,
    disk_schema: Option<Value>,
    embedded_schema: Option<Value>,
}

impl Catalog {
    pub fn get(&self, adapter: Option<&str>) -> Result<&FrozenContract, LoadError> {
        let adapter = adapter.filter(|a| !a.is_empty()).ok_or(LoadError::MissingAdapter)?;
        if let Some(error) = self.rejected.get(adapter) {
            return Err(error.clone());
        }
        self.entries.get(adapter).ok_or_else(|| LoadError::MissingDeclaration {
            adapter: adapter.to_string(),
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &FrozenContract)> {
        self.entries.iter().map(|(name, contract)| (name.as_str(), contract))
    }

    pub fn rejected(&self) -> &BTreeMap<String, LoadError> {
        &self.rejected
    }
}

/// Inputs are already parsed JSON values. None is an explicit missing-disk error, as in the CLI
/// load_agent_spec path. File read/parse errors remain caller errors, never embedded fallback.
/// Parsed non-object values retain CLI get(adapter) semantics (no key => embedded fallback).
pub fn load_catalog(
    disk: Option<&Value>,
    embedded: &Value,
    policy_epoch: u64,
    origin: RegistrationOrigin,
    native_backend: bool,
) -> Result<Catalog, LoadError> {
    let disk = disk.ok_or(LoadError::MissingDiskDocument)?;
    let disk_candidates: BTreeMap<_, _> = crate::governance::agent_candidates_from_json(disk)
        .into_iter().collect();
    let embedded_candidates: BTreeMap<_, _> =
        crate::governance::agent_candidates_from_json(embedded).into_iter().collect();
    let names: BTreeSet<String> = disk.as_object().into_iter().flat_map(|o| o.keys())
        .chain(embedded.as_object().into_iter().flat_map(|o| o.keys()))
        .filter(|name| !name.starts_with('_')).cloned().collect();
    let mut catalog = Catalog { entries: BTreeMap::new(), rejected: BTreeMap::new(),
        disk_schema:disk.get("_schema").cloned(),embedded_schema:embedded.get("_schema").cloned() };
    for adapter in names {
        let loaded = load_one(
            &adapter, disk, embedded, &disk_candidates, &embedded_candidates,
            policy_epoch, origin, native_backend,
        );
        match loaded {
            Ok(contract) => { catalog.entries.insert(adapter, contract); }
            Err(error) => { catalog.rejected.insert(adapter, error); }
        }
    }
    Ok(catalog)
}

#[allow(clippy::too_many_arguments)]
fn load_one(
    adapter: &str,
    disk: &Value,
    embedded: &Value,
    disk_candidates: &BTreeMap<String, String>,
    embedded_candidates: &BTreeMap<String, String>,
    policy_epoch: u64,
    origin: RegistrationOrigin,
    native_backend: bool,
) -> Result<FrozenContract, LoadError> {
    let source = if disk.get(adapter).is_some() {
        DeclarationSource::Disk
    } else {
        DeclarationSource::Embedded
    };
    let effective_launch_spec = cli_effective_spec(disk, embedded, adapter)?;
    let command = effective_launch_spec.get("cmd").and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| LoadError::MissingEffectiveCommand { adapter: adapter.to_string() })?
        .to_string();
    let shipped_command = embedded.get(adapter).and_then(|v| v.get("cmd"))
        .and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_owned);
    let legacy_bin = match source {
        DeclarationSource::Disk => disk_candidates.get(adapter),
        DeclarationSource::Embedded => embedded_candidates.get(adapter),
    };
    let builtin = crate::receiver::BUILTIN_ADAPTERS.contains(&adapter);
    let exact_shipped_command = shipped_command.as_deref() == Some(command.as_str());
    let recipe = native_recipe(&command, builtin && exact_shipped_command, &effective_launch_spec);
    if builtin && shipped_command.is_none() {
        return Err(LoadError::MissingEmbeddedBuiltin { adapter: adapter.to_string() });
    }
    let mut notices = Vec::new();
    if builtin && !exact_shipped_command {
        let (legacy_class, _) = crate::receiver::classify_policy(
            adapter, legacy_bin.map(String::as_str).unwrap_or(""),
            embedded_candidates.get(adapter).map(String::as_str), native_backend,
        );
        notices.push(DeclarationNotice::ChangedBuiltinCommand { legacy_class });
    }
    let (class, weak_risk) = if !native_backend {
        (ProofClass::ObservedAgentWeak, Some(WeakRisk::NoNativeBackend))
    } else if builtin && exact_shipped_command {
        (ProofClass::FullRequired, None)
    } else if builtin {
        (ProofClass::ObservedAgentWeak, Some(WeakRisk::CustomWrapperReceiverIdentity))
    } else {
        (ProofClass::ObservedAgentWeak, Some(WeakRisk::DeclaredCustomInvocation))
    };
    // Exactly agent_env_pairs value behavior: absent/null/non-object => empty, discard
    // non-string values, sort pairs. No inheritance from embedded inside a disk adapter.
    let launch_env_pairs = cli_agent_env_pairs(&effective_launch_spec);
    let launch_prompt_markers = cys::agent_markers::marker_candidates(effective_launch_spec.get("prompt_marker"));
    let launch_composer_placeholder = effective_launch_spec.get("composer_placeholder")
        .and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_owned);
    let receiver_prompt_markers = crate::governance::merged_prompt_marker(disk, embedded, adapter);
    let receiver_composer_placeholder = crate::governance::merged_composer_placeholder(disk, embedded, adapter);
    let disk_record = disk.get(adapter).cloned();
    let embedded_record = embedded.get(adapter).cloned();
    // Epoch/origin/backend are lifecycle/platform inputs, not config bytes. The immutable policy
    // separately carries the chosen epoch/origin/class. Array and command-string order is exact.
    let digest_input = json!({
        "schema": "cys-receiver-config-v1",
        "disk_schema": disk.get("_schema"),
        "embedded_schema": embedded.get("_schema"),
        "adapter": adapter,
        "command": command,
        "effective_launch_spec": effective_launch_spec,
        "launch_env_pairs": launch_env_pairs,
        "launch_prompt_markers": launch_prompt_markers,
        "launch_composer_placeholder": launch_composer_placeholder,
        "receiver_prompt_markers": receiver_prompt_markers,
        "receiver_composer_placeholder": receiver_composer_placeholder,
        "disk_record": disk_record,
        "embedded_record": embedded_record,
    });
    let config_digest = raw_config_digest(&digest_input);
    Ok(FrozenContract {
        policy: FrozenPolicy {
            policy_epoch,
            declaration: command, // no trim, split/rejoin, expansion or normalization
            config_digest,
            adapter: adapter.to_string(),
            recipe,
            origin,
            class,
            weak_risk,
        },
        declaration_source: source,
        launch_env_pairs,
        effective_launch_spec,
        launch_prompt_markers,
        launch_composer_placeholder,
        receiver_prompt_markers,
        receiver_composer_placeholder,
        disk_record,
        embedded_record,
        notices,
    })
}

/// Pure value counterpart of CLI load_agent_spec/fill_missing_fields; CLI is NOT refactored.
fn cli_effective_spec(disk: &Value, embedded: &Value, adapter: &str) -> Result<Value, LoadError> {
    let embedded_record = embedded.get(adapter);
    let mut resolved = disk.get(adapter).or(embedded_record).cloned()
        .ok_or_else(|| LoadError::MissingDeclaration { adapter: adapter.to_string() })?;
    if let Some(fields) = resolved.as_object_mut() {
        for key in ["ready_marker", "approval_patterns", cys::first_run_gates::ADAPTER_KEY,
                    "prompt_marker", "composer_placeholder", "hooks_inject_directive"] {
            if !fields.contains_key(key) {
                if let Some(value) = embedded_record.and_then(|spec| spec.get(key)) {
                    fields.insert(key.to_string(), value.clone());
                }
            }
        }
    }
    cys::agent_markers::promote_stale_vendor_defaults(adapter, &mut resolved, embedded_record);
    Ok(resolved)
}

fn cli_agent_env_pairs(spec: &Value) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = spec.get("env").and_then(Value::as_object)
        .into_iter().flat_map(|fields| fields.iter())
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect();
    pairs.sort();
    pairs
}

fn declaration_words(command: &str) -> Result<Vec<String>, RecipeGap> {
    if command.chars().any(|c| matches!(c, '$' | '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '\n' | '\r')) {
        return Err(RecipeGap::ShellSyntax);
    }
    if command.contains('=') && command.contains(['\'', '"', '\\']) {
        return Err(RecipeGap::AssignmentProvenance);
    }
    let needs_lexer = command.contains(['\'', '"']) || (!cfg!(windows) && command.contains('\\'));
    let words = if needs_lexer {
        if cfg!(windows) || command.ends_with('\\') { return Err(RecipeGap::PlatformQuoting); }
        let words = cys::approval_tokenize(command).ok_or(RecipeGap::UnclosedQuote)?;
        if words.first().is_some_and(|word| word.contains('\\')) {
            return Err(RecipeGap::LiteralPosixBackslash);
        }
        // '=' plus quote/escape was refused above: no decoded token may be skipped as env.
        words
    } else {
        let assignment = |word: &str| word.split_once('=').is_some_and(|(key, _)| {
            !key.is_empty() && !key.as_bytes()[0].is_ascii_digit()
                && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        });
        command.split_whitespace().skip_while(|word| assignment(word)).map(str::to_owned).collect()
    };
    if words.first().is_none_or(|word| word.is_empty()) { return Err(RecipeGap::MissingExecutable); }
    Ok(words)
}

fn resume_patterns(spec: &Value) -> Vec<Vec<crate::receiver::DeclaredArgument>> {
    use crate::receiver::DeclaredArgument;
    ["resume_arg", "resume_arg_fallback"].into_iter().filter_map(|key| {
        let text = spec.get(key)?.as_str()?;
        // These are comparison patterns only, never evaluated or sent as commands.
        if text.chars().any(|c| matches!(c, '$' | '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '\n' | '\r')) { return None; }
        if cfg!(windows) && text.contains(['\'', '"', '\\']) { return None; }
        let words = cys::approval_tokenize(text)?;
        if words.is_empty() { return None; }
        words.into_iter().map(|word| {
            if let Some((prefix, suffix)) = word.split_once("{session_id}") {
                if suffix.contains("{session_id}") { return None; }
                Some(DeclaredArgument::SessionId { prefix:prefix.as_bytes().to_vec(), suffix:suffix.as_bytes().to_vec() })
            } else { Some(DeclaredArgument::Literal(word.into_bytes())) }
        }).collect::<Option<Vec<_>>>()
    }).collect()
}

fn native_recipe(command: &str, shipped_builtin: bool, spec: &Value) -> NativeRecipe {
    use crate::receiver::{DeclaredInvocation, generic_interpreter_recipe, is_script_runtime, runtime_script_slot};
    let words = match declaration_words(command) {
        Ok(words) => words, Err(gap) => return NativeRecipe::Unsupported(gap),
    };
    let bin = words[0].rsplit(['/', '\\']).next().unwrap_or(&words[0]).to_string();
    if bin.is_empty() { return NativeRecipe::Unsupported(RecipeGap::MissingExecutable); }
    let argv: Vec<Vec<u8>> = words.into_iter().map(String::into_bytes).collect();
    let declared_invocation = if is_script_runtime(&bin) {
        let Some(index) = runtime_script_slot(&argv) else { return NativeRecipe::Unsupported(RecipeGap::MissingScript); };
        let script = &argv[index];
        if script.is_empty() { return NativeRecipe::Unsupported(RecipeGap::MissingScript); }
        // Bun subcommands are not direct script paths. Do not infer a new grammar from a match.
        if bin.trim_end_matches(".exe").eq_ignore_ascii_case("bun") {
            let direct_path = script.contains(&b'/') || script.contains(&b'\\')
                || [b".js".as_slice(),b".mjs",b".cjs",b".ts",b".mts",b".cts",b".tsx",b".jsx"]
                    .iter().any(|suffix| script.ends_with(suffix));
            if !direct_path { return NativeRecipe::Unsupported(RecipeGap::MissingScript); }
        }
        Some(DeclaredInvocation { argv,script_index:Some(index),allowed_suffixes:resume_patterns(spec) })
    } else if generic_interpreter_recipe(&bin) {
        return NativeRecipe::Unsupported(RecipeGap::UnsupportedInterpreter);
    } else if !shipped_builtin {
        // Even an unenumerated interpreter cannot discard an available declared script/argument.
        // Flexible builtin/native/npm recipes remain unchanged; custom programs retain exact args.
        Some(DeclaredInvocation { argv,script_index:None,allowed_suffixes:resume_patterns(spec) })
    } else { None };
    NativeRecipe::Supported(Recipe { bin,declared_invocation })
}

/// Canonicalization is JSON key ordering only, not shell/string normalization. Scalars use
/// serde_json encoding; arrays keep order; object keys sort by Rust String ordering (UTF-8).
fn canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i != 0 { out.push(','); }
                canonical_json(item, out);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push('{');
            let mut keys: Vec<_> = fields.keys().collect();
            keys.sort_unstable();
            for (i, key) in keys.into_iter().enumerate() {
                if i != 0 { out.push(','); }
                out.push_str(&serde_json::to_string(key).expect("JSON string serialization"));
                out.push(':');
                canonical_json(&fields[key], out);
            }
            out.push('}');
        }
        _ => out.push_str(&serde_json::to_string(value).expect("JSON value serialization")),
    }
}

fn raw_config_digest(value: &Value) -> String {
    let mut canonical = String::new();
    canonical_json(value, &mut canonical);
    // Despite its name, this helper performs RAW SHA-256 on supplied bytes; it does not normalize.
    // Do not replace with digest_text(), which collapses whitespace even inside JSON strings.
    cys::mission_gate::digest_normalized(&canonical)
}

// Explicit private serialization only. Catalog/FrozenContract/FrozenPolicy themselves do not
// implement Serialize or Debug, so an ordinary event cannot accidentally expose their contents.
#[derive(serde::Serialize)]
struct PrivateRecipe<'a> {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    supported: Option<&'a Recipe>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsupported: Option<&'static str>,
}
fn private_recipe(recipe:&NativeRecipe)->PrivateRecipe<'_> {
    match recipe {
        NativeRecipe::Supported(recipe)=>PrivateRecipe { kind:"supported",supported:Some(recipe),unsupported:None },
        NativeRecipe::Unsupported(gap)=>PrivateRecipe { kind:"unsupported",supported:None,unsupported:Some(match gap {
            RecipeGap::UnclosedQuote=>"unclosed_quote",RecipeGap::ShellSyntax=>"shell_syntax",
            RecipeGap::PlatformQuoting=>"platform_quoting",RecipeGap::MissingExecutable=>"missing_executable",
            RecipeGap::AssignmentProvenance=>"assignment_provenance",RecipeGap::LiteralPosixBackslash=>"literal_posix_backslash",
            RecipeGap::UnsupportedInterpreter=>"unsupported_interpreter",RecipeGap::MissingScript=>"missing_script",
        }) },
    }
}
#[derive(serde::Serialize)]
struct PrivatePolicy<'a> {
    policy_epoch:u64,declaration:&'a str,config_digest:&'a str,adapter:&'a str,
    recipe:PrivateRecipe<'a>,origin:RegistrationOrigin,class:ProofClass,weak_risk:Option<WeakRisk>,
}
#[derive(serde::Serialize)]
struct PrivateContract<'a> {
    policy:PrivatePolicy<'a>,declaration_source:&'static str,
    launch_env_pairs:&'a [(String,String)],effective_launch_spec:&'a Value,
    launch_prompt_markers:&'a [String],launch_composer_placeholder:Option<&'a str>,
    receiver_prompt_markers:Option<&'a [String]>,receiver_composer_placeholder:Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_record:Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedded_record:Option<&'a Value>,
    legacy_class_notices:Vec<ProofClass>,
}
#[derive(serde::Serialize)]
struct PrivateRejection<'a> { adapter:&'a str,kind:&'static str,referred_adapter:Option<&'a str> }
fn private_rejection<'a>(adapter:&'a str,error:&'a LoadError)->PrivateRejection<'a> {
    let (kind,referred_adapter)=match error {
        LoadError::MissingDiskDocument=>("missing_disk_document",None),
        LoadError::MissingAdapter=>("missing_adapter",None),
        LoadError::MissingDeclaration { adapter }=>("missing_declaration",Some(adapter.as_str())),
        LoadError::MissingEmbeddedBuiltin { adapter }=>("missing_embedded_builtin",Some(adapter.as_str())),
        LoadError::MissingEffectiveCommand { adapter }=>("missing_effective_command",Some(adapter.as_str())),
    };
    PrivateRejection { adapter,kind,referred_adapter }
}
#[derive(serde::Serialize)]
struct PrivateCatalog<'a> {
    schema:&'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_schema:Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedded_schema:Option<&'a Value>,
    entries:Vec<PrivateContract<'a>>,rejected:Vec<PrivateRejection<'a>>,
}
#[derive(serde::Serialize)]
struct PrivateIdentityEntry<'a> {
    adapter:&'a str,config_digest:&'a str,declaration:&'a str,recipe:PrivateRecipe<'a>,
    class:ProofClass,weak_risk:Option<WeakRisk>,
}
#[derive(serde::Serialize)]
struct PrivateCatalogIdentity<'a> {
    schema:&'static str,entries:Vec<PrivateIdentityEntry<'a>>,rejected:Vec<PrivateRejection<'a>>,
}
impl Catalog {
    pub(crate) fn declaration_count(&self)->usize { self.entries.len().saturating_add(self.rejected.len()) }
    /// Exact frozen private values, including Unsupported recipes. Use a bounded serializer
    /// owned by the persistence layer. This data is never an executable recovered capability.
    pub(crate) fn for_private_persistence(&self)->impl serde::Serialize + '_ {
        PrivateCatalog { schema:"cys-receiver-catalog-v1",disk_schema:self.disk_schema.as_ref(),
            embedded_schema:self.embedded_schema.as_ref(),entries:self.entries.values().map(|c| {
                let p=&c.policy;
                PrivateContract { policy:PrivatePolicy { policy_epoch:p.policy_epoch,declaration:&p.declaration,
                    config_digest:&p.config_digest,adapter:&p.adapter,recipe:private_recipe(&p.recipe),
                    origin:p.origin,class:p.class,weak_risk:p.weak_risk },
                    declaration_source:match c.declaration_source { DeclarationSource::Disk=>"disk",DeclarationSource::Embedded=>"embedded" },
                    launch_env_pairs:&c.launch_env_pairs,effective_launch_spec:&c.effective_launch_spec,
                    launch_prompt_markers:&c.launch_prompt_markers,launch_composer_placeholder:c.launch_composer_placeholder.as_deref(),
                    receiver_prompt_markers:c.receiver_prompt_markers.as_deref(),receiver_composer_placeholder:c.receiver_composer_placeholder.as_deref(),
                    disk_record:c.disk_record.as_ref(),embedded_record:c.embedded_record.as_ref(),
                    legacy_class_notices:c.notices.iter().map(|n| match n { DeclarationNotice::ChangedBuiltinCommand { legacy_class }=>*legacy_class }).collect() }
            }).collect(),rejected:self.rejected.iter().map(|(a,e)|private_rejection(a,e)).collect() }
    }
    /// Private identity input for same-contract registration coalescing. Epoch and origin are
    /// deliberately excluded, just as ReceiverCell::register_known preserves them on a no-op.
    /// This view still contains private declaration/recipe values; hash it, never publish it.
    pub(crate) fn private_contract_identity(&self)->impl serde::Serialize + '_ {
        PrivateCatalogIdentity { schema:"cys-receiver-catalog-contract-v1",entries:self.entries.values().map(|c| {
            let p=&c.policy;PrivateIdentityEntry { adapter:&p.adapter,config_digest:&p.config_digest,
                declaration:&p.declaration,recipe:private_recipe(&p.recipe),class:p.class,weak_risk:p.weak_risk }
        }).collect(),rejected:self.rejected.iter().map(|(a,e)|private_rejection(a,e)).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embed() -> Value {
        json!({"_schema":3,
            "codex":{"cmd":"codex --no-daemon", "env":{"ACCOUNT":"vendor"},
                "ready_marker":"READY", "approval_patterns":[{"name":"approve","pattern":"Allow"}],
                "first_run_gates":{"source":"builtin"}, "prompt_marker":["›","»"],
                "composer_placeholder":"Ask Codex to do anything", "hooks_inject_directive":false,
                "inject_delay_secs":12, "clear_cmd":"/new", "notes":"vendor"},
            "claude":{"cmd":"claude --dangerously-skip-permissions", "env":{"ACCOUNT":"vendor"}, "prompt_marker":"❯"}
        })
    }

    fn catalog(disk: &Value, embedded: &Value) -> Catalog {
        load_catalog(Some(disk), embedded, 7, RegistrationOrigin::ClaimRole, true).unwrap()
    }

    #[test]
    fn cli_absent_adapter_uses_embedded_but_existing_object_inherits_only_six_fields() {
        let embedded = embed();
        let absent = catalog(&json!({}), &embedded);
        let inherited = absent.get(Some("codex")).unwrap();
        assert_eq!(inherited.effective_launch_spec, embedded["codex"]);
        assert_eq!(inherited.launch_env_pairs, vec![("ACCOUNT".into(), "vendor".into())]);
        let disk = json!({"codex":{"cmd":"codex --no-daemon", "notes":"mine"}});
        let before = disk.clone();
        let present = catalog(&disk, &embedded);
        let entry = present.get(Some("codex")).unwrap();
        assert_eq!(entry.policy.class, ProofClass::FullRequired);
        assert!(entry.launch_env_pairs.is_empty());
        for forbidden in ["env", "inject_delay_secs", "clear_cmd"] {
            assert!(entry.effective_launch_spec.get(forbidden).is_none(), "unexpected inheritance of {forbidden}");
        }
        for key in ["ready_marker", "approval_patterns", "first_run_gates", "prompt_marker", "composer_placeholder", "hooks_inject_directive"] {
            assert_eq!(entry.effective_launch_spec[key], embedded["codex"][key]);
        }
        assert_eq!(entry.effective_launch_spec["notes"], json!("mine"));
        assert_eq!(disk, before);
    }

    #[test]
    fn existing_missing_null_or_wrong_cmd_is_error_not_embedded_command_fallback() {
        let embedded = embed();
        for record in [json!({}), json!({"cmd":null}), json!({"cmd":9}), json!(null), json!("bad-adapter")] {
            let c = catalog(&json!({"codex":record}), &embedded);
            assert!(matches!(c.get(Some("codex")), Err(LoadError::MissingEffectiveCommand { .. })));
        }
        assert!(matches!(load_catalog(None, &embedded, 1, RegistrationOrigin::ClaimRole, true), Err(LoadError::MissingDiskDocument)));
        // A parsed non-object root is not a missing file: CLI get(adapter) misses, then uses embedded.
        assert_eq!(catalog(&Value::Null, &embedded).get(Some("codex")).unwrap().effective_launch_spec, embedded["codex"]);
        assert!(matches!(catalog(&json!({}), &embedded).get(None), Err(LoadError::MissingAdapter)));
    }

    #[test]
    fn cli_explicit_ui_null_survives_while_receiver_merger_is_separately_named() {
        let embedded = embed();
        let disk = json!({"codex":{"cmd":"codex --no-daemon", "ready_marker":null,
            "approval_patterns":null, "first_run_gates":null, "prompt_marker":null,
            "composer_placeholder":null, "hooks_inject_directive":null}});
        let c = catalog(&disk, &embedded);
        let entry = c.get(Some("codex")).unwrap();
        for key in ["ready_marker", "approval_patterns", "first_run_gates", "prompt_marker", "composer_placeholder", "hooks_inject_directive"] {
            assert_eq!(entry.effective_launch_spec[key], Value::Null);
        }
        assert!(entry.launch_prompt_markers.is_empty());
        assert!(entry.launch_composer_placeholder.is_none());
        assert_eq!(entry.receiver_prompt_markers, Some(vec!["›".into(), "»".into()]));
        assert_eq!(entry.receiver_composer_placeholder.as_deref(), Some("Ask Codex to do anything"));
    }

    #[test]
    fn cli_env_pairs_filter_sort_and_never_inherit_inside_disk_adapter() {
        let embedded = embed();
        for env in [Value::Null, json!(7), json!("not-map"), json!([])] {
            let c = catalog(&json!({"codex":{"cmd":"codex --no-daemon", "env":env}}), &embedded);
            assert!(c.get(Some("codex")).unwrap().launch_env_pairs.is_empty());
        }
        let disk = json!({"codex":{"cmd":"codex --no-daemon", "env":{"Z":"z","A":"a","BAD":false,"NUMBER":7}}});
        let c = catalog(&disk, &embedded);
        let entry = c.get(Some("codex")).unwrap();
        assert_eq!(entry.launch_env_pairs, vec![("A".into(), "a".into()),("Z".into(), "z".into())]);
        assert_eq!(entry.effective_launch_spec["env"]["BAD"], json!(false), "raw effective spec is retained");
        assert_eq!(entry.policy.class, ProofClass::FullRequired);
        assert_ne!(entry.policy.config_digest, catalog(&json!({}), &embedded).get(Some("codex")).unwrap().policy.config_digest);
    }

    #[test]
    fn quote_or_escape_provenance_cannot_skip_a_literal_assignment_looking_executable() {
        let embedded = embed();
        for command in ["'ACCOUNT=one' codex", r"ACCOUNT\=one codex", r"ACCO\UNT=one codex", "ACCOUNT='one two' codex"] {
            let c = catalog(&json!({"codex":{"cmd":command}}), &embedded);
            let entry = c.get(Some("codex")).unwrap();
            assert_eq!(entry.policy.declaration, command);
            assert_eq!(entry.policy.class, ProofClass::ObservedAgentWeak);
            assert_eq!(entry.policy.recipe, NativeRecipe::Unsupported(RecipeGap::AssignmentProvenance));
            assert!(entry.policy.native_policy().is_none());
            assert!(!c.rejected().contains_key("codex"));
        }
        let genuine = catalog(&json!({"codex":{"cmd":"ACCOUNT=one codex"}}), &embedded);
        assert_eq!(genuine.get(Some("codex")).unwrap().policy.native_policy().unwrap().recipe.bin, "codex");
    }

    #[test]
    fn exact_command_class_and_unsupported_custom_population_remain_frozen() {
        let embedded = embed();
        for command in ["codex --different-mode", " codex --no-daemon", "codex  --no-daemon", "wrapper && codex", "$(choose) codex"] {
            let c = catalog(&json!({"codex":{"cmd":command}}), &embedded);
            let entry = c.get(Some("codex")).unwrap();
            assert_eq!(entry.policy.class, ProofClass::ObservedAgentWeak);
            assert_eq!(entry.policy.weak_risk, Some(WeakRisk::CustomWrapperReceiverIdentity));
            assert_eq!(entry.policy.declaration, command);
            assert_eq!(entry.policy.origin, RegistrationOrigin::ClaimRole);
        }
        let custom = catalog(&json!({"mine":{"cmd":"wrapper && custom"}}), &embedded);
        assert_eq!(custom.get(Some("mine")).unwrap().policy.weak_risk, Some(WeakRisk::DeclaredCustomInvocation));
        assert!(custom.get(Some("mine")).unwrap().policy.native_policy().is_none());
    }

    #[test]
    fn stale_scalar_promotion_is_after_cli_field_fill_and_explicit_array_is_preserved() {
        let embedded = embed();
        for (marker, expected) in [(json!("›"), vec!["›".to_string(),"»".to_string()]), (json!(["›"]), vec!["›".to_string()])] {
            let c = catalog(&json!({"codex":{"cmd":"codex --no-daemon", "prompt_marker":marker}}), &embedded);
            assert_eq!(c.get(Some("codex")).unwrap().launch_prompt_markers, expected);
        }
    }

    #[test]
    fn frozen_values_and_config_digest_do_not_follow_later_mutation_or_origin_changes() {
        let mut embedded = embed();
        let c = catalog(&json!({}), &embedded);
        let before = c.get(Some("codex")).unwrap().policy.config_digest.clone();
        let no_native = load_catalog(Some(&json!({})), &embedded, 99, RegistrationOrigin::HandObserved, false).unwrap();
        assert_eq!(no_native.get(Some("codex")).unwrap().policy.weak_risk, Some(WeakRisk::NoNativeBackend));
        assert_eq!(no_native.get(Some("codex")).unwrap().policy.config_digest, before);
        embedded["codex"]["cmd"] = json!("changed");
        assert_eq!(c.get(Some("codex")).unwrap().policy.declaration, "codex --no-daemon");
        assert_eq!(c.get(Some("codex")).unwrap().policy.config_digest, before);
        assert_eq!(raw_config_digest(&json!({"a":"two  spaces","b":2})), raw_config_digest(&serde_json::from_str::<Value>(r#"{"b":2,"a":"two  spaces"}"#).unwrap()));
        assert_ne!(raw_config_digest(&json!("two  spaces")), raw_config_digest(&json!("two spaces")));
    }

    #[test]
    fn native_bridge_debug_and_public_receipts_omit_raw_synthetic_values() {
        // Synthetic sentinels only. This test does not assert any real credential was emitted.
        let command_sentinel = "SYNTHETIC_PRIVATE_COMMAND_VALUE_71";
        let env_sentinel = "SYNTHETIC_PRIVATE_ENV_VALUE_92";
        let embedded = embed();
        let disk = json!({"codex":{"cmd":format!("codex --token {command_sentinel}"), "env":{"TOKEN":env_sentinel}}});
        let c = catalog(&disk, &embedded);
        let entry = c.get(Some("codex")).unwrap();
        let native = entry.policy.native_policy().unwrap();
        for public in [
            format!("{native:?}"),
            serde_json::to_string(&native.public_summary()).unwrap(),
            serde_json::to_string(&entry.policy.public_summary()).unwrap(),
            serde_json::to_string(&json!({"event":"receiver.policy", "policy":native.public_summary()})).unwrap(),
        ] {
            assert!(!public.contains(command_sentinel));
            assert!(!public.contains(env_sentinel));
            assert!(public.contains(&entry.policy.config_digest));
        }
        let private = serde_json::to_value(native.for_private_persistence()).unwrap();
        assert_eq!(private["declaration"], json!(format!("codex --token {command_sentinel}")), "private persistence must retain authority bytes");
        let restored: ReceiverPolicy = serde_json::from_value(private).unwrap();
        assert_eq!(restored, native);
        // ReceiverPolicy itself no longer implements Serialize; public callers must project it.
    }

    #[cfg(unix)]
    #[test]
    fn c8policy_literal_posix_backslash_is_not_a_different_native_basename() {
        let embedded = embed();
        for command in [r"'/tmp/agent\name' --flag", r"/tmp/agent\\name --flag"] {
            let c = catalog(&json!({"custom":{"cmd":command}}), &embedded);
            let p = &c.get(Some("custom")).unwrap().policy;
            assert_eq!(p.declaration, command);
            assert_eq!(p.class, ProofClass::ObservedAgentWeak);
            assert!(p.native_policy().is_none(), "literal POSIX backslash must not certify a different basename");
            assert!(!c.rejected().contains_key("custom"));
        }
    }

    fn invocation(executable: &str, argv: &[&str]) -> cys::process_observation::ProcessObservation {
        cys::process_observation::ProcessObservation {
            identity: cys::process_observation::Identity { pid:21,parent:20,process_group:21,
                birth_seconds:10,birth_microseconds:2,pointer_width:8,executable:executable.as_bytes().to_vec() },
            invocation:cys::process_observation::Invocation { launch_path:executable.as_bytes().to_vec(),
                argv:argv.iter().map(|a|a.as_bytes().to_vec()).collect() },
        }
    }

    #[test]
    fn c8policy_declared_js_script_cannot_match_a_bare_or_different_interpreter() {
        for runtime in ["node","nodejs","bun"] {
            let command=format!("{runtime} --require /setup/init.js /agents/my-agent.js --mode assist");
            let c=catalog(&json!({"custom":{"cmd":command}}),&json!({}));
            let p=&c.get(Some("custom")).unwrap().policy;
            assert_eq!(p.class,ProofClass::ObservedAgentWeak);
            let native=p.native_policy().expect("enumerated JS script declaration must retain a positive native recipe");
            let exe=format!("/usr/bin/{runtime}");
            for args in [vec![runtime],vec![runtime,"/different/my-agent.js"],
                vec![runtime,"--eval","/agents/my-agent.js"],
                vec![runtime,"--require","/agents/my-agent.js","/other.js"],
                vec![runtime,"--require","/setup/init.js","/other/my-agent.js","--mode","assist"],
                vec![runtime,"--require","/wrong/init.js","/agents/my-agent.js","--mode","assist"],
                vec![runtime,"--require","/setup/init.js","/agents/my-agent.js","--mode","other"],
                vec![runtime,"--require","/setup/init.js","/agents/my-agent.js","--mode","assist","--extra"]] {
                assert_eq!(crate::receiver::match_slot(&native.recipe,&invocation(&exe,&args)),None,
                    "declared script absence or contradiction must not match the interpreter itself: {args:?}");
            }
            let actual=[runtime,"--require","/setup/init.js","/agents/my-agent.js","--mode","assist"];
            assert!(matches!(crate::receiver::match_slot(&native.recipe,&invocation(&exe,&actual)),Some(crate::receiver::Slot::Script(3))),
                "actual declared script must remain a positive witness");
        }
    }

    #[test]
    fn c8policy_unenumerated_interpreter_recipes_stay_declared_without_bare_native_alias() {
        for command in ["python /agents/my-agent.py","python3 /agents/my-agent.py",
            "bash -lc 'exec /agents/my-agent'","sh /agents/my-agent.sh","node","node --eval code"] {
            let c=catalog(&json!({"custom":{"cmd":command}}),&json!({}));
            let p=&c.get(Some("custom")).unwrap().policy;
            assert_eq!(p.class,ProofClass::ObservedAgentWeak);
            assert_eq!(p.declaration,command);
            assert!(p.native_policy().is_none(),"unsupported declared interpreter cannot become a bare native recipe: {command}");
            assert!(!c.rejected().contains_key("custom"));
        }
    }

    #[test]
    fn c8policy_constraints_preserve_declared_resume_suffixes_and_runtime_aliases() {
        let c=catalog(&json!({"custom":{"cmd":"nodejs /agents/a.js --mode assist",
            "resume_arg":"resume {session_id}","resume_arg_fallback":"resume --last"}}),&json!({}));
        let p=c.get(Some("custom")).unwrap().policy.native_policy().unwrap();
        for suffix in [vec![],vec!["resume","session-123"],vec!["resume","--last"]] {
            let mut argv=vec!["nodejs","/agents/a.js","--mode","assist"];argv.extend(suffix);
            assert_eq!(crate::receiver::match_slot(&p.recipe,&invocation("/usr/bin/node",&argv)),Some(crate::receiver::Slot::Script(1)),
                "declared Node alias/fresh/resume invocation must remain supported");
        }
        for suffix in [vec!["resume",""],vec!["--continue"],vec!["resume","id","extra"]] {
            let mut argv=vec!["nodejs","/agents/a.js","--mode","assist"];argv.extend(suffix);
            assert_eq!(crate::receiver::match_slot(&p.recipe,&invocation("/usr/bin/node",&argv)),None,
                "undeclared or empty resume suffix must not be guessed");
        }
        assert_eq!(crate::receiver::match_slot(&p.recipe,&invocation("/usr/bin/bun",&["bun","/agents/a.js","--mode","assist"])),None);
        let encoded=serde_json::to_value(p.for_private_persistence()).unwrap();
        let restored:ReceiverPolicy=serde_json::from_value(encoded).unwrap();
        assert_eq!(restored,p,"private persistence must retain invocation and resume constraints");
        assert!(!serde_json::to_string(&p.public_summary()).unwrap().contains("/agents/a.js"));
    }

    #[test]
    fn c8policy_custom_program_arguments_are_not_discarded_for_unenumerated_runtimes() {
        let c=catalog(&json!({"custom":{"cmd":"ruby /agents/a.rb --mode assist"}}),&json!({}));
        let p=c.get(Some("custom")).unwrap().policy.native_policy().unwrap();
        assert_eq!(p.class,ProofClass::ObservedAgentWeak);
        for argv in [vec!["ruby"],vec!["ruby","/other/a.rb","--mode","assist"],
            vec!["ruby","/agents/a.rb","--mode","other"]] {
            assert_eq!(crate::receiver::match_slot(&p.recipe,&invocation("/usr/bin/ruby",&argv)),None,
                "custom argument contradiction cannot be hidden by an unenumerated runtime name");
        }
        assert_eq!(crate::receiver::match_slot(&p.recipe,&invocation("/usr/bin/ruby",&["ruby","/agents/a.rb","--mode","assist"])),Some(crate::receiver::Slot::Native));
    }

    #[test]
    fn c8policy_legacy_interpreter_recipe_and_malformed_constraint_cannot_bypass_loader() {
        for bin in ["node","nodejs","bun","python","python3.12","bash","powershell.exe"] {
            let recipe:Recipe=serde_json::from_value(json!({"bin":bin})).unwrap();
            assert_eq!(crate::receiver::match_slot(&recipe,&invocation(bin,&[bin])),None,
                "missing declaration constraint must not restore known interpreter equality");
        }
        let c=catalog(&json!({"custom":{"cmd":"node /agents/a.js"}}),&json!({}));
        let mut recipe=c.get(Some("custom")).unwrap().policy.native_policy().unwrap().recipe;
        recipe.declared_invocation.as_mut().unwrap().script_index=Some(0);
        assert_eq!(crate::receiver::match_slot(&recipe,&invocation("node",&["node","/agents/a.js"])),None);
        let recipe=c.get(Some("custom")).unwrap().policy.native_policy().unwrap().recipe;
        assert_eq!(crate::receiver::match_slot(&recipe,&invocation("node",&["","/agents/a.js"])),None);
    }

    #[test]
    fn c8archive_private_image_reconstructs_eligible_contract_inputs() {
        let disk=json!({"_schema":17,"custom":{"cmd":"python /agents/a.py",
            "env":{"TOKEN":"SYNTHETIC_ARCHIVE_PRIVATE_VALUE"},"prompt_marker":null}});
        let embedded=json!({"_schema":23,"claude":{"cmd":"claude --dangerously-skip-permissions","prompt_marker":"prompt"}});
        let c=catalog(&disk,&embedded);
        let image=serde_json::to_value(c.for_private_persistence()).unwrap();
        assert_eq!(image["schema"],"cys-receiver-catalog-v1");
        assert_eq!(image["entries"].as_array().unwrap().len(),c.entries().count(),
            "private archive must retain unsupported declarations");
        let mut restored_disk=json!({});let mut restored_embedded=json!({});
        for (key,dest) in [("disk_schema",&mut restored_disk),("embedded_schema",&mut restored_embedded)] {
            if let Some(value)=image.get(key) { dest["_schema"]=value.clone(); }
        }
        for entry in image["entries"].as_array().unwrap() {
            let adapter=entry["policy"]["adapter"].as_str().unwrap();
            if let Some(record)=entry.get("disk_record") { restored_disk[adapter]=record.clone(); }
            if let Some(record)=entry.get("embedded_record") { restored_embedded[adapter]=record.clone(); }
        }
        let restored=catalog(&restored_disk,&restored_embedded);
        for (adapter,entry) in c.entries() {
            let other=restored.get(Some(adapter)).unwrap();
            assert_eq!(other.policy.config_digest,entry.policy.config_digest,
                "archive omitted an input needed to reconstruct the frozen contract");
            assert_eq!(other.policy.declaration,entry.policy.declaration);
            assert_eq!(other.launch_env_pairs,entry.launch_env_pairs);
            assert!(!serde_json::to_string(&entry.policy.public_summary()).unwrap().contains("SYNTHETIC_ARCHIVE_PRIVATE_VALUE"));
        }
        let custom=image["entries"].as_array().unwrap().iter().find(|e|e["policy"]["adapter"]=="custom").unwrap();
        assert_eq!(custom["policy"]["recipe"]["kind"],"unsupported");
        assert_eq!(custom["policy"]["recipe"]["unsupported"],"unsupported_interpreter");
        assert_eq!(custom["launch_env_pairs"][0][1],"SYNTHETIC_ARCHIVE_PRIVATE_VALUE");
        assert!(c.get(Some("custom")).unwrap().policy.native_policy().is_none());
    }

    #[test]
    fn c8archive_contract_identity_excludes_lifecycle_but_preserves_configuration_and_class() {
        let disk=json!({"claude":{"cmd":"claude --dangerously-skip-permissions","env":{"A":"one"}}});
        let embedded=embed();
        let a=load_catalog(Some(&disk),&embedded,1,RegistrationOrigin::ClaimRole,true).unwrap();
        let b=load_catalog(Some(&disk),&embedded,99,RegistrationOrigin::Reclaim,true).unwrap();
        let identity=|c:&Catalog|serde_json::to_vec(&c.private_contract_identity()).unwrap();
        assert_eq!(identity(&a),identity(&b),"same contract registration must not be changed by origin/epoch alone");
        assert_ne!(serde_json::to_vec(&a.for_private_persistence()).unwrap(),serde_json::to_vec(&b.for_private_persistence()).unwrap(),
            "the private image must still retain actual captured origin and epoch");
        let mut changed=disk.clone();changed["claude"]["env"]["A"]=json!("two");
        assert_ne!(identity(&a),identity(&catalog(&changed,&embedded)),"environment changes are part of contract identity");
        let weak=load_catalog(Some(&disk),&embedded,1,RegistrationOrigin::ClaimRole,false).unwrap();
        assert_ne!(identity(&a),identity(&weak),"proof class must remain in contract identity");
    }

    #[test]
    fn c8archive_private_encoding_obeys_the_callers_byte_limit() {
        struct Limited { used:usize,limit:usize }
        impl std::io::Write for Limited {
            fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {
                if bytes.len()>self.limit-self.used { return Err(std::io::Error::other("fixture archive cap")); }
                self.used+=bytes.len();Ok(bytes.len())
            }
            fn flush(&mut self)->std::io::Result<()> { Ok(()) }
        }
        let disk=json!({"custom":{"cmd":"custom-agent","env":{"LARGE":"x".repeat(50000)}}});
        let c=catalog(&disk,&json!({}));let mut out=Limited { used:0,limit:64 };
        assert!(serde_json::to_writer(&mut out,&c.for_private_persistence()).is_err(),"oversized private catalog must not report successful bounded encoding");
        assert!(out.used<=out.limit);
    }

    #[test]
    fn c8archive_schema_absence_null_and_rejected_names_remain_distinct_data() {
        let absent=catalog(&json!({}),&embed());
        let null=catalog(&json!({"_schema":null}),&embed());
        assert!(serde_json::to_value(absent.for_private_persistence()).unwrap().get("disk_schema").is_none());
        assert_eq!(serde_json::to_value(null.for_private_persistence()).unwrap().get("disk_schema"),Some(&Value::Null));
        let rejected=catalog(&json!({"bad":{"cmd":null}}),&json!({}));
        assert_eq!(rejected.declaration_count(),1);
        let image=serde_json::to_value(rejected.for_private_persistence()).unwrap();
        assert_eq!(image["entries"],json!([]));
        assert_eq!(image["rejected"][0]["adapter"],"bad");
        assert_eq!(image["rejected"][0]["kind"],"missing_effective_command");
    }
}
