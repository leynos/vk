# OrthoConfig User's Guide

`OrthoConfig` is a Rust library that unifies command‑line arguments,
environment variables and configuration files into a single, strongly typed
configuration struct. It is inspired by tools such as `esbuild` and is designed
to minimize boiler‑plate. The library uses `serde` for deserialization and
`clap` for argument parsing, while `figment` provides layered configuration
management. This guide covers the functionality currently implemented in the
repository.

## Core concepts and motivation

Rust projects often wire together `clap` for CLI parsing, `serde` for
de/serialization, and ad‑hoc code for loading `*.toml` files or reading
environment variables. Mapping between different naming conventions (kebab‑case
flags, `UPPER_SNAKE_CASE` environment variables, and `snake_case` struct
fields) can be tedious. `OrthoConfig` addresses these problems by letting
developers describe their configuration once and then automatically loading
values from multiple sources. The core features are:

- **Layered configuration** – Configuration values can come from application
  defaults, configuration files, environment variables and command‑line
  arguments. Later sources override earlier ones. Command‑line arguments have
  the highest precedence and defaults the lowest.

- **Orthographic naming** – A single field in a Rust struct is automatically
  mapped to a CLI flag (kebab‑case), an environment variable (upper snake case
  with a prefix), and a file key (snake case). This removes the need for manual
  aliasing.

- **Type‑safe deserialization** – Values are deserialized into strongly typed
  Rust structs using `serde`.

- **Easy adoption** – A procedural macro `#[derive(OrthoConfig)]` adds the
  necessary code. Developers only need to derive `serde` traits on their
  configuration struct and call a generated method to load the configuration.

- **Customizable behaviour** – Attributes such as `default`, `cli_long`,
  `cli_short` and `merge_strategy` provide fine‑grained control over naming and
  merging behaviour.

## Installation and dependencies

Add `ortho_config` as a dependency in `Cargo.toml` along with `serde`:

```toml
[dependencies]
ortho_config = "0.3.0"            # replace with the latest version
serde = { version = "1.0", features = ["derive"] }
clap = { version = "4", features = ["derive"] }    # required for CLI support
```

By default, only TOML configuration files are supported. To enable JSON5
(`.json` and `.json5`) and YAML (`.yaml` and `.yml`) support, enable the
corresponding cargo features:

```toml
[dependencies]
ortho_config = { version = "0.3.0", features = ["json5", "yaml"] }
```

Enabling the `json5` feature causes both `.json` and `.json5` files to be
parsed using the JSON5 format. Without this feature, these files are ignored
during discovery and do not cause errors if present. The `yaml` feature
similarly enables `.yaml` and `.yml` files; without it, such files are skipped
during discovery and do not cause errors if present.

## Migrating from earlier versions

Projects using a pre‑0.3 release can upgrade with the following steps:

- `#[derive(OrthoConfig)]` remains the correct way to annotate configuration
  structs. No additional derives are required.
- Remove any `load_with_reference_fallback` helpers. The merge logic inside
  `load_and_merge_subcommand_for` supersedes this workaround.
- Replace calls to deprecated helpers such as `load_subcommand_config_for` with
  `ortho_config::subcommand::load_and_merge_subcommand_for`.

Each subcommand struct can expose a wrapper method that forwards to
`load_and_merge_subcommand_for`:

```rust
use ortho_config::{subcommand::load_and_merge_subcommand_for, OrthoConfig,
                   OrthoError};
use serde::Deserialize;

#[derive(Deserialize, OrthoConfig)]
struct PrArgs {
    reference: String,
}

impl PrArgs {
    fn load_and_merge(cli: &Cli) -> Result<Self, OrthoError> {
        load_and_merge_subcommand_for::<Self>(cli)
    }
}
```

After parsing the top‑level `Cli` struct, call `PrArgs::load_and_merge(&cli)`
to obtain the merged configuration for that subcommand.

## Defining configuration structures

A configuration is represented by a plain Rust struct. To take advantage of
`OrthoConfig`, derive the following traits:

- `serde::Deserialize` and `serde::Serialize` – required for deserializing
  values and merging overrides.

- The derive macro generates a hidden `clap::Parser` implementation, so
  manual `clap` annotations are not required in typical use. CLI customization
  is performed using `ortho_config` attributes such as `cli_short`, or
  `cli_long`.

- `OrthoConfig` – provided by the library. This derive macro generates the code
  to load and merge configuration from multiple sources.

Optionally, the struct can include a `#[ortho_config(prefix = "PREFIX")]`
attribute. The prefix sets a common string for environment variables and
configuration file names. Trailing underscores are trimmed and the prefix is
lower‑cased when used to form file names. For example, a prefix of `APP_`
results in environment variables like `APP_PORT` and file names such as
`.app.toml`.

### Field‑level attributes

Field attributes modify how a field is sourced or merged:

| Attribute                   | Behaviour                                                                                                                                                                     |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `default = expr`            | Supplies a default value when no source provides one. The expression can be a literal or a function path.                                                                     |
| `cli_long = "name"`         | Overrides the automatically generated long CLI flag (kebab‑case).                                                                                                             |
| `cli_short = 'c'`           | Adds a single‑letter short flag for the field.                                                                                                                                |
| `merge_strategy = "append"` | For `Vec<T>` fields, specifies that values from different sources should be concatenated. This is currently the only supported strategy and is the default for vector fields. |

Unrecognized keys are ignored by the derive macro for forwards compatibility.
Unknown keys will therefore silently do nothing. Developers who require
stricter validation may add manual `compile_error!` guards.

By default, each field receives a long flag derived from its name in kebab-case
and a short flag from its first letter. If that letter is already used, the
macro assigns the upper-case variant to the next field. Further collisions
require specifying `cli_short` explicitly. Short flags must be ASCII
alphanumeric and may not use clap's global `-h` or `-V` options. Long flags
must contain only ASCII alphanumeric characters, hyphens or underscores and
cannot be named `help` or `version`.

### Example configuration struct

The following example illustrates many of these features:

```rust
  use ortho_config::{OrthoConfig, OrthoError};
  use serde::{Deserialize, Serialize};

  #[derive(Debug, Clone, Deserialize, Serialize, OrthoConfig)]
  #[ortho_config(prefix = "APP")]                // environment variables start with APP_
  struct AppConfig {
      /// Logging verbosity
      log_level: String,

    /// Port to bind on – defaults to 8080 when unspecified
    #[ortho_config(default = 8080)]
    port: u16,

    /// Optional list of features. Values from files, environment and CLI are appended.
    #[ortho_config(merge_strategy = "append")]
    features: Vec<String>,

    /// Nested configuration for the database. A separate prefix is used to avoid ambiguity.
    #[serde(flatten)]
    database: DatabaseConfig,

    /// Enable verbose output; also available as -v via cli_short
    #[ortho_config(cli_short = 'v')]
    verbose: bool,
  }

#[derive(Debug, Clone, Deserialize, Serialize, OrthoConfig)]
#[ortho_config(prefix = "DB")]               // used in conjunction with APP_ prefix to form APP_DB_URL
struct DatabaseConfig {
    url: String,

    #[ortho_config(default = 5)]
    pool_size: Option<u32>,
}

fn main() -> Result<(), OrthoError> {
    // Parse CLI arguments and merge with defaults, file and environment
    let config = AppConfig::load()?;
    println!("Final config: {:#?}", config);
    Ok(())
}
```

`clap` attributes are not required in general; flags are derived from field
names and `ortho_config` attributes. In this example, the `AppConfig` struct
uses a prefix of `APP`. The `DatabaseConfig` struct declares a prefix `DB`,
resulting in environment variables such as `APP_DB_URL`. The `features` field
is a `Vec<String>` and accumulates values from multiple sources rather than
overwriting them.

## Loading configuration and precedence rules

### How loading works

The `load_from_iter` method (used by the convenience `load`) performs the
following steps:

1. Builds a `figment` configuration profile. A defaults provider constructed
   from the `#[ortho_config(default = …)]` attributes is added first.

2. Attempts to load a configuration file. Candidate file paths are searched in
   the following order:

   1. A `--config-path` CLI argument. A hidden option is generated
      automatically by the derive macro; if the user defines a `config_path`
      field in their struct then that will override the hidden option.
      Alternatively the environment variable `PREFIXCONFIG_PATH` (or
      `CONFIG_PATH` if no prefix is set) can specify an explicit file.

   1. A dotfile named `.config.toml` or `.<prefix>.toml` in the current working
      directory.

   1. A dotfile of the same name in the user's home directory.

   1. On Unix‑like systems, the XDG configuration directory (e.g.
      `~/.config/app/config.toml`) is searched using the `xdg` crate; on
      Windows, the `%APPDATA%` and `%LOCALAPPDATA%` directories are checked.

   1. If the `json5` or `yaml` features are enabled, files with `.json`,
      `.json5`, `.yaml` or `.yml` extensions are also considered in these
      locations.

3. Adds an environment provider using the prefix specified on the struct. Keys
   are upper‑cased and nested fields use double underscores (`__`) to separate
   components.

4. Adds a provider containing the CLI values (captured as `Option<T>` fields)
   as the final layer.

5. Merges vector fields according to the `merge_strategy` (currently only
   `append`) so that lists of values from lower precedence sources are extended
   with values from higher precedence ones.

6. Attempts to extract the merged configuration into the concrete struct. On
   success it returns the completed configuration; otherwise an `OrthoError` is
   returned.

### Source precedence

Values are loaded from each layer in a specific order. Later layers override
earlier ones. The precedence, from lowest to highest, is:

1. **Application‑defined defaults** – values provided via `default` attributes
   or `Option<T>` fields are considered defaults.

2. **Configuration file** – values from a TOML (or JSON5/YAML) file loaded from
   one of the paths listed above.

3. **Environment variables** – variables prefixed with the struct's `prefix`
   (e.g. `APP_PORT`, `APP_DATABASE__URL`) override file values.

4. **Command‑line arguments** – values parsed by `clap` override all other
   sources.

Nested structs are flattened in the environment namespace by joining field
names with double underscores. For example, if `AppConfig` has a nested
`database` field and the prefix is `APP`, then `APP_DATABASE__URL` sets the
`database.url` field. If a nested struct has its own prefix attribute, that
prefix is used for its fields (e.g. `APP_DB_URL`).

When `clap`'s `flatten` attribute is employed to compose argument groups, the
flattened struct is initialized even if no CLI flags within the group are
specified. During merging, `ortho_config` discards these empty groups so that
values from configuration files or the environment remain in place unless a
field is explicitly supplied on the command line.

### Using defaults and optional fields

Fields of type `Option<T>` are treated as optional values. If no source
provides a value for an `Option<T>` field then it remains `None`. To provide a
default value for a non‑`Option` field or for an `Option<T>` field that should
have an initial value, specify `#[ortho_config(default = expr)]`. This default
acts as the lowest‑precedence source and is overridden by file, environment or
CLI values.

### Environment variable naming

Environment variables are upper‑cased and use underscores. The struct‑level
prefix (if supplied) is prepended without any separator, and nested fields are
separated by double underscores. For the `AppConfig` and `DatabaseConfig`
example above, valid environment variables include `APP_LOG_LEVEL`, `APP_PORT`,
`APP_DATABASE__URL` and `APP_DATABASE__POOL_SIZE`. If the nested struct has its
own prefix (`DB`), then the environment variable becomes `APP_DB_URL`.

Comma-separated values such as `DDLINT_RULES=A,B,C` are parsed as lists. The
loader converts these strings into arrays before merging, so array fields
behave the same across environment variables, CLI arguments and configuration
files. Values containing literal commas must be wrapped in quotes or brackets
to disable list parsing.

## Configuration inheritance

A configuration file may specify an `extends` key pointing to another file. The
referenced file is loaded first and the current file's values override it. The
path is resolved relative to the file containing the `extends` directive.
Precedence across all sources becomes base file → extending file → environment
variables → CLI flags. Cycles are detected and reported via a `CyclicExtends`
error. Prefix handling and subcommand namespaces work as normal when
inheritance is in use.

## Dynamic rule tables

Map fields such as `BTreeMap<String, RuleConfig>` allow configuration files to
declare arbitrary rule keys. Any table nested under `rules.<name>` is
deserialized into the map without prior knowledge of the key names. This
enables use cases like:

```toml
[rules.consistent-casing]
enabled = true
[rules.no-tabs]
enabled = false
```

Each entry becomes a map key with its associated struct value.

## Ignore patterns

Lists of files or directories to exclude can be specified via comma-separated
environment variables and CLI flags. Values are merged using the `append`
strategy, so that configuration defaults are extended by environment variables
and finally by the CLI. Whitespace around entries is trimmed and duplicates are
preserved. For example:

```bash
DDLINT_IGNORE_PATTERNS=".git/,build/"
mytool --ignore-patterns target/
```

results in `ignore_patterns = [".git/", "build/", "target/"]`.

## Subcommand configuration

Many CLI applications use `clap` subcommands to perform different operations.
`OrthoConfig` supports per‑subcommand defaults via a dedicated `cmds`
namespace. The helper function `load_and_merge_subcommand_for` loads defaults
for a specific subcommand and merges them beneath the CLI values. The older
`load_subcommand_config` and `load_subcommand_config_for` helpers are
deprecated in favour of this function. The merged struct is returned as a new
instance; the original `cli` struct remains unchanged. CLI fields left unset
(`None`) do not override environment or file defaults, avoiding accidental loss
of configuration.

### How it works

When a struct derives `OrthoConfig`, it also implements the associated
`prefix()` method. This method returns the configured prefix string.
`load_and_merge_subcommand_for(prefix, cli_struct)` uses this prefix to build a
`cmds.<subcommand>` section name for the configuration file and an
`PREFIX_CMDS_SUBCOMMAND_` prefix for environment variables. Configuration is
loaded in the same order as global configuration (defaults → file → environment
→ CLI), but only values in the `[cmds.<subcommand>]` section or environment
variables beginning with `PREFIX_CMDS_<SUBCOMMAND>_` are considered.

### Example

Suppose an application has a `pr` subcommand that accepts a `reference`
argument and a `repo` global option. With `OrthoConfig` the argument structures
might be defined as follows:

```rust
use clap::Parser;
use ortho_config::{OrthoConfig, load_and_merge_subcommand_for};
use serde::{Deserialize, Serialize};

#[derive(Parser, Deserialize, Serialize, Debug, OrthoConfig, Clone, Default)]
#[ortho_config(prefix = "VK")]               // all variables start with VK
pub struct GlobalArgs {
    pub repo: Option<String>,
}

#[derive(Parser, Deserialize, Serialize, Debug, OrthoConfig, Clone, Default)]
#[ortho_config(prefix = "VK")]               // subcommands share the same prefix
pub struct PrArgs {
    #[arg(required = true)]
    pub reference: Option<String>,            // optional for merging defaults but required on the CLI
}

fn main() -> Result<(), ortho_config::OrthoError> {
    let cli_pr = PrArgs::parse();
    // Merge defaults from [cmds.pr] and VK_CMDS_PR_* over CLI
    let merged_pr = load_and_merge_subcommand_for::<PrArgs>(&cli_pr)?;
    println!("PrArgs after merging: {:#?}", merged_pr);
    Ok(())
}
```

A configuration file might include:

```toml
[cmds.pr]
reference = "https://github.com/leynos/mxd/pull/31"

[cmds.issue]
reference = "https://github.com/leynos/mxd/issues/7"
```

and environment variables could override these defaults:

```bash
VK_CMDS_PR_REFERENCE=https://github.com/owner/repo/pull/42
VK_CMDS_ISSUE_REFERENCE=https://github.com/owner/repo/issues/101
```

Within the `vk` example repository, the global `--repo` option is provided via
the `GlobalArgs` struct. A developer can set this globally using the
environment variable `VK_REPO` without passing `--repo` on every invocation.
Subcommands `pr` and `issue` load their defaults from the `cmds` namespace and
environment variables. If the `reference` field is missing in the defaults, the
tool continues using the CLI value instead of exiting with an error.

### Dispatching with `clap‑dispatch`

The `clap‑dispatch` crate can be combined with `OrthoConfig` to simplify
subcommand execution. Each subcommand struct implements a trait defining the
action to perform. An enum of subcommands is annotated with
`#[clap_dispatch(fn run(...))]`, and the `load_and_merge_subcommand_for`
function can be called on each variant before dispatching. See the
`Subcommand Configuration` section of the `OrthoConfig` [README](../README.md)
for a complete example.

## Error handling

`load` and `load_and_merge_subcommand_for` return a `Result<T, OrthoError>`.
`OrthoError` wraps errors from `clap`, file I/O and `figment`. Failures during
the final merge of CLI values over configuration sources surface as the `Merge`
variant, providing clearer diagnostics when the combined data is invalid. When
multiple sources fail, the errors are collected into the `Aggregate` variant so
callers can inspect each individual failure. Consumers should handle these
errors appropriately, for example by printing them to stderr and exiting. If
required fields are missing after merging, the crate returns
`OrthoError::MissingRequiredValues` with a user‑friendly list of missing paths
and hints on how to provide them. For example:

```text
Missing required values:
  sample_value (use --sample-value, SAMPLE_VALUE, or file entry)
```

## Additional notes

- **Vector merging** – For `Vec<T>` fields the default merge strategy is
  `append`, meaning that values from the configuration file appear first, then
  environment variables and finally CLI arguments. The
  `merge_strategy = "append"` attribute can be used for clarity, though it is
  implied.

- **Option&lt;T&gt; fields** – Fields of type `Option<T>` are not treated as
  required. They default to `None` and can be set via any source. Required CLI
  arguments can be represented as `Option<T>` to allow configuration defaults
  while still requiring the CLI to provide a value when defaults are absent;
  see the `vk` example above.

- **Config path flag** – The derive macro inserts a hidden `--config-path`
  option into the CLI to override the configuration file path. To expose or
  rename this flag, define your own `config_path` field with a `cli_long`
  attribute:

  ```rust
  #[derive(ortho_config::OrthoConfig)]
  struct AppConfig {
      #[serde(skip)]
      #[ortho_config(cli_long = "config")]
      config_path: Option<std::path::PathBuf>,
  }
  ```

  The example above enables `--config` and the `CONFIG_PATH` environment
  variable. The option remains hidden from help output unless a `config_path`
  field is declared.

- **Changing naming conventions** – Currently, only the default
  snake/kebab/upper snake mappings are supported. Future versions may introduce
  attributes such as `file_key` or `env` to customize names further.

- **Testing** – Because the CLI and environment variables are merged at
  runtime, integration tests should set environment variables and construct CLI
  argument vectors to exercise the merge logic. The `figment` crate makes it
  easy to inject additional providers when writing unit tests.

- **Sanitized providers** – The `sanitized_provider` helper returns a `Figment`
  provider with `None` fields removed. It aids manual layering when bypassing
  the derive macro. For example:

  ```rust
  use figment::{Figment, providers::Serialized};
  use ortho_config::sanitized_provider;

  let fig = Figment::from(Serialized::defaults(&Defaults::default()))
      .merge(sanitized_provider(&cli)?);
  let cfg: Defaults = fig.extract()?;
  ```

## Conclusion

`OrthoConfig` streamlines configuration management in Rust applications. By
defining a single struct and annotating it with a small number of attributes,
developers obtain a full configuration parser that respects CLI arguments,
environment variables and configuration files with predictable precedence.
Subcommand support and integration with `clap‑dispatch` further reduce
boiler‑plate in complex CLI tools. The example `vk` repository demonstrates how
a real application can adopt `OrthoConfig` to handle global options and
subcommand defaults. Contributions to the project are welcome, and the design
documents outline planned improvements such as richer error messages and
support for additional naming strategies.
