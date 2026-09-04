//! Host boundary and command contract for the native Stack CLI.

#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use stack_engine::{
    CheckOutput, Diagnostic, Engine, FormatOutput, OperationalError, ProviderAsset, ProviderNotice,
    ProviderPack, RenderOutput, Severity,
};

mod config;
mod provider;
mod provider_catalog;
mod templates;

/// Exit status used when a command completes without Stack error diagnostics.
pub const EXIT_SUCCESS: u8 = 0;
/// Exit status used when Stack source contains at least one error diagnostic.
pub const EXIT_STACK_ERROR: u8 = 1;
/// Exit status used for argument, host I/O, or engine operational failures.
pub const EXIT_USAGE_OR_IO: u8 = 2;

const GENERAL_HELP: &str = "\
Stack diagram toolchain

Usage:
  stack <COMMAND> [OPTIONS]
  stack help [COMMAND]

Commands:
  init       Create a Stack file from a versioned starter template
  check      Validate a Stack source file without modifying it
  fmt        Format a file in place or read from standard input
  render     Render standalone SVG to standard output or a file
  icons      List catalogs and import audited provider icon archives
  help       Print this message or the help of a subcommand
  version    Print version information

Options:
  -h, --help          Print help
  -v, -V, --version   Print version

Examples:
  stack init
  stack init --template application-and-data -o architecture.stack
  stack check arch.stack
  stack fmt --check arch.stack
  stack render arch.stack -o arch.svg
  stack icons list aws s3
";
const INIT_HELP: &str = "\
Create a Stack file from a versioned starter template

Usage:
  stack init [--template <TEMPLATE>] [-o <FILE>] [--force]

Options:
  --template <TEMPLATE>  Select a curated template; defaults to hello-stack
  -o, --output <FILE>    Write to this path; defaults to diagram.stack
  --force                Replace an existing output file atomically
  -h, --help             Print help

Templates:
  hello-stack                Smallest directed diagram using built-in icons
  application-and-data       Application, database, details, and edge labels
  groups-and-layout          Groups, ordering, rank, and layout direction
  commerce-platform         Production-like built-in architecture
  aws-serverless-checkout    AWS serverless architecture
  gcp-data-service           Google Cloud data service
  azure-event-platform       Azure event-driven platform
  github-delivery-workflow   GitHub delivery workflow
  mixed-provider-platform    AWS, GCP, Azure, and SaaS architecture

Provider templates remain valid without imported icon packs and render with
deterministic fallback icons. Use 'stack icons import <PROVIDER> --accept-terms'
before rendering when branded icons are required.

Examples:
  stack init
  stack init --template groups-and-layout
  stack init --template aws-serverless-checkout -o checkout.stack
  stack init --force
";
const CHECK_HELP: &str = "\
Validate a Stack source file without modifying it

Usage:
  stack check <FILE>

Arguments:
  <FILE>  Read Stack source bytes from this file

Options:
  -h, --help  Print help

Examples:
  stack check arch.stack
";
const FORMAT_HELP: &str = "\
Format Stack source canonically

Usage:
  stack fmt <FILE>
  stack fmt --check <FILE>
  stack fmt -

Arguments:
  <FILE>  Format the file atomically in place
  -       Read from standard input and write to standard output

Options:
  --check     Report whether formatting is required without writing output
  -h, --help  Print help

Examples:
  stack fmt arch.stack
  stack fmt --check arch.stack
  stack fmt - < input.stack > output.stack
";
const RENDER_HELP: &str = "\
Render Stack source as standalone SVG

Usage:
  stack render <FILE> [--provider-pack <DIRECTORY>] [-o <OUTPUT>] [--notice <NOTICE>]

Arguments:
  <FILE>                      Read Stack source bytes from this file

Options:
  --provider-pack <DIRECTORY> Read known provider packs from this icon-store root
  -o <OUTPUT>                 Write SVG atomically instead of using standard output
  --notice <NOTICE>           Write exact used-provider notices atomically
  -h, --help                  Print help

Default icon store:
  $XDG_CONFIG_HOME/stack/icons or $HOME/.config/stack/icons

Examples:
  stack render arch.stack
  stack render arch.stack -o arch.svg
  stack render arch.stack --notice arch.NOTICE.md -o arch.svg
";
const ICONS_HELP: &str = "\
Manage local provider icon packs

Usage:
  stack icons <COMMAND>

Commands:
  list    List searchable asset-free provider catalog metadata
  import  Download and import an audited provider icon archive
  help    Print this message or the help of an icons subcommand

Options:
  -h, --help  Print help

Providers:
  aws            305 AWS Architecture Icons
  gcp             45 Google Cloud product and category icons
  azure          639 Azure Public Service Icons
  simple-icons    62 curated developer and collaboration tools

Examples:
  stack icons list
  stack icons list aws s3
  stack icons import gcp --accept-terms
";
const ICONS_LIST_HELP: &str = "\
List searchable asset-free provider catalog metadata

Usage:
  stack icons list
  stack icons list <PROVIDER> [QUERY]

Arguments:
  <PROVIDER>  aws, gcp, azure, or simple-icons
  [QUERY]     Case-insensitive ID, product, or category substring

Options:
  -h, --help  Print help

Examples:
  stack icons list
  stack icons list aws s3
";
const ICONS_IMPORT_HELP: &str = "\
Download and import audited provider icon archives

Usage:
  stack icons import <PROVIDER> --accept-terms [-o <DIRECTORY>]

Arguments:
  <PROVIDER>  aws, gcp, azure, or simple-icons

Options:
  --accept-terms  Confirm that you reviewed all provider and brand terms
  -o <DIRECTORY>  Store packs below this icon-store root
  -h, --help       Print help

Default icon store:
  $XDG_CONFIG_HOME/stack/icons or $HOME/.config/stack/icons

Examples:
  stack icons import gcp --accept-terms
  stack icons import simple-icons --accept-terms -o .stack-icons
";
const HELP_HELP: &str = "\
Print top-level or subcommand help

Usage:
  stack help
  stack help <COMMAND>
  stack help icons <COMMAND>

Arguments:
  <COMMAND>  init, check, fmt, render, icons, help, or version

Examples:
  stack help
  stack help render
  stack help icons import
";
const VERSION_HELP: &str = "\
Print Stack CLI version information

Usage:
  stack version

Options:
  -h, --help  Print help

Examples:
  stack version
";
const MAX_PROVIDER_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_ASSET_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormatMode {
    Write,
    Check,
}

#[derive(Debug, PartialEq, Eq)]
enum RenderDestination {
    Stdout,
    File(PathBuf),
}

type ImportProvider = fn(&str, &Path) -> Result<provider::ImportSummary, provider::ImportError>;

/// Runs the CLI with explicit streams and returns its process exit status.
pub fn run(
    arguments: impl IntoIterator<Item = OsString>,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        return argument_error("missing command", stderr);
    };

    if is_help_flag(&command) {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(GENERAL_HELP, stdout, stderr);
    }
    if is_version_flag(&command) {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_version(stdout, stderr);
    }
    if command == OsStr::new("help") {
        return run_help(arguments, stdout, stderr);
    }
    if command == OsStr::new("version") {
        return run_version(arguments, stdout, stderr);
    }
    if command == OsStr::new("init") {
        return run_init(arguments, stdout, stderr);
    }
    if command == OsStr::new("check") {
        return run_check(arguments, stdout, stderr);
    }
    if command == OsStr::new("fmt") {
        return run_format(arguments, stdin, stdout, stderr);
    }
    if command == OsStr::new("render") {
        return run_render(arguments, stdout, stderr);
    }
    if command == OsStr::new("icons") {
        return run_icons(&mut arguments, stdout, stderr);
    }

    unknown_command_error(
        "stack",
        &command,
        &["init", "check", "fmt", "render", "icons", "help", "version"],
        "stack help",
        stderr,
    )
}

fn run_help(
    mut arguments: impl Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(command) = arguments.next() else {
        return write_stdout(GENERAL_HELP, stdout, stderr);
    };
    if is_help_flag(&command) {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(HELP_HELP, stdout, stderr);
    }
    if command == OsStr::new("icons") {
        return run_icons_help(&mut arguments, stdout, stderr);
    }

    let help = if command == OsStr::new("init") {
        INIT_HELP
    } else if command == OsStr::new("check") {
        CHECK_HELP
    } else if command == OsStr::new("fmt") {
        FORMAT_HELP
    } else if command == OsStr::new("render") {
        RENDER_HELP
    } else if command == OsStr::new("help") {
        HELP_HELP
    } else if command == OsStr::new("version") {
        VERSION_HELP
    } else {
        return unknown_command_error(
            "stack help",
            &command,
            &["init", "check", "fmt", "render", "icons", "help", "version"],
            "stack help",
            stderr,
        );
    };

    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }
    write_stdout(help, stdout, stderr)
}

fn run_init(
    mut arguments: impl Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let first = arguments.next();
    if first
        .as_ref()
        .is_some_and(|argument| is_help_flag(argument))
    {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(INIT_HELP, stdout, stderr);
    }

    let mut template_id = None;
    let mut destination = None;
    let mut force = false;
    let mut remaining = first.into_iter().chain(arguments);
    while let Some(option) = remaining.next() {
        if option == OsStr::new("--template") {
            if template_id.is_some() {
                return argument_error("duplicate '--template' option", stderr);
            }
            let Some(value) = remaining.next() else {
                return argument_error("missing template ID after '--template'", stderr);
            };
            if value.to_string_lossy().starts_with('-') {
                return argument_error("missing template ID after '--template'", stderr);
            }
            template_id = Some(value);
        } else if option == OsStr::new("-o") || option == OsStr::new("--output") {
            if destination.is_some() {
                return argument_error("duplicate output option", stderr);
            }
            let Some(value) = remaining.next() else {
                return argument_error("missing output file after output option", stderr);
            };
            if value.to_string_lossy().starts_with('-') {
                return argument_error("missing output file after output option", stderr);
            }
            destination = Some(PathBuf::from(value));
        } else if option == OsStr::new("--force") {
            if force {
                return argument_error("duplicate '--force' option", stderr);
            }
            force = true;
        } else if option.to_string_lossy().starts_with('-') {
            return argument_error(
                &format!("unknown option '{}'", option.to_string_lossy()),
                stderr,
            );
        } else {
            return argument_error(
                &format!("unexpected argument '{}'", option.to_string_lossy()),
                stderr,
            );
        }
    }

    let template_id = template_id.unwrap_or_else(|| OsString::from(templates::DEFAULT_ID));
    let Some(template) = templates::find(&template_id) else {
        let rendered_id = template_id.to_string_lossy();
        let available = templates::ALL
            .iter()
            .map(|template| template.id)
            .collect::<Vec<_>>()
            .join(", ");
        return argument_error(
            &format!("unknown template '{rendered_id}'; available templates: {available}"),
            stderr,
        );
    };
    let destination = destination.unwrap_or_else(|| PathBuf::from("diagram.stack"));
    initialize_file(&destination, force, stdout, stderr, template)
}

fn initialize_file(
    destination: &Path,
    force: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    template: templates::Template,
) -> u8 {
    let result = if force {
        atomic_write_output(destination, template.source)
    } else {
        create_new_file(destination, template.source)
    };
    if let Err(error) = result {
        if !force && error.kind() == io::ErrorKind::AlreadyExists {
            return write_stderr_error(
                &format!(
                    "cannot create '{}': already exists; pass '--force' to replace it",
                    destination.display()
                ),
                stderr,
            );
        }
        return write_stderr_error(
            &format!(
                "cannot {} '{}': {}",
                if force { "write" } else { "create" },
                destination.display(),
                stable_io_error(error.kind())
            ),
            stderr,
        );
    }

    let mut message = format!(
        "Created '{}' from template '{}'.\n\nNext:\n  stack check {}\n  stack render {} -o diagram.svg\n",
        destination.display(),
        template.id,
        destination.display(),
        destination.display()
    );
    if !template.providers.is_empty() {
        let _ = writeln!(
            message,
            "\nProvider icons: {}",
            template.providers.join(", ")
        );
        for provider in template.providers {
            let _ = writeln!(message, "  stack icons import {provider} --accept-terms");
        }
        message.push_str("Without imported packs, render uses deterministic fallback icons.\n");
    }
    write_stdout(&message, stdout, stderr)
}

fn run_version(
    mut arguments: impl Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(argument) = arguments.next() else {
        return write_version(stdout, stderr);
    };
    if is_help_flag(&argument) {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(VERSION_HELP, stdout, stderr);
    }
    argument_error(
        &format!("unexpected argument '{}'", argument.to_string_lossy()),
        stderr,
    )
}

fn run_icons(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(command) = arguments.next() else {
        return argument_error("missing command for 'stack icons'", stderr);
    };
    if is_help_flag(&command) {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_HELP, stdout, stderr);
    }
    if command == OsStr::new("help") {
        return run_icons_help(arguments, stdout, stderr);
    }
    if command == OsStr::new("list") {
        return run_icons_list(arguments, stdout, stderr);
    }
    if command != OsStr::new("import") {
        return unknown_command_error(
            "stack icons",
            &command,
            &["list", "import", "help"],
            "stack help icons",
            stderr,
        );
    }
    run_icons_import(arguments, stdout, stderr)
}

fn run_icons_help(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(command) = arguments.next() else {
        return write_stdout(ICONS_HELP, stdout, stderr);
    };
    if is_help_flag(&command) {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_HELP, stdout, stderr);
    }
    if command == OsStr::new("help") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_HELP, stdout, stderr);
    }
    let help = if command == OsStr::new("list") {
        ICONS_LIST_HELP
    } else if command == OsStr::new("import") {
        ICONS_IMPORT_HELP
    } else {
        return unknown_command_error(
            "stack icons",
            &command,
            &["list", "import", "help"],
            "stack help icons",
            stderr,
        );
    };
    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }
    write_stdout(help, stdout, stderr)
}

fn run_icons_list(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let first = arguments.next();
    if first
        .as_ref()
        .is_some_and(|value| value == OsStr::new("--help") || value == OsStr::new("-h"))
    {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_LIST_HELP, stdout, stderr);
    }
    let query = arguments.next();
    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }
    let provider = first.as_ref().map(|value| value.to_string_lossy());
    let query = query.as_ref().map(|value| value.to_string_lossy());
    match provider::render_catalog_list(provider.as_deref(), query.as_deref()) {
        Ok(output) => write_stdout(&output, stdout, stderr),
        Err(error) => write_stderr_error(&format!("cannot list provider icons: {error}"), stderr),
    }
}

fn run_icons_import(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    run_icons_import_with(
        arguments,
        stdout,
        stderr,
        import_provider_from_official_sources,
    )
}

fn run_icons_import_with(
    arguments: &mut dyn Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    import: ImportProvider,
) -> u8 {
    let Some(provider) = arguments.next() else {
        return argument_error("missing provider for 'stack icons import'", stderr);
    };
    if provider == OsStr::new("--help") || provider == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(ICONS_IMPORT_HELP, stdout, stderr);
    }
    if provider.to_string_lossy().starts_with('-') {
        return argument_error(
            &format!("unknown option '{}'", provider.to_string_lossy()),
            stderr,
        );
    }
    let Some(provider_name) = provider.to_str() else {
        return argument_error("provider must be valid UTF-8", stderr);
    };
    if !provider_catalog::PROVIDER_IDS.contains(&provider_name) {
        return argument_error(
            &format!(
                "unknown provider '{provider_name}'; expected aws, gcp, azure, or simple-icons"
            ),
            stderr,
        );
    }

    let mut accepted_terms = false;
    let mut output_root = None;
    while let Some(option) = arguments.next() {
        if option == OsStr::new("--accept-terms") {
            if accepted_terms {
                return argument_error("duplicate '--accept-terms' option", stderr);
            }
            accepted_terms = true;
        } else if option == OsStr::new("-o") {
            if output_root.is_some() {
                return argument_error("duplicate '-o' option", stderr);
            }
            let Some(path) = arguments.next() else {
                return argument_error("missing output directory after '-o'", stderr);
            };
            output_root = Some(PathBuf::from(path));
        } else {
            return argument_error(
                &format!("unexpected argument '{}'", option.to_string_lossy()),
                stderr,
            );
        }
    }
    if !accepted_terms {
        return argument_error(
            "provider terms must be reviewed and accepted with '--accept-terms'",
            stderr,
        );
    }
    let environment = config::Environment::capture();
    let icon_store_root = match config::icon_store_root(output_root.as_deref(), &environment) {
        Ok(path) => path,
        Err(error) => return write_stderr_error(&error, stderr),
    };
    let output = icon_store_root.join(provider_name);
    let result = import(provider_name, &output);
    match result {
        Ok(summary) => write_stdout(
            &format!(
                "Imported {} {} icons to '{}'.\nManifest: {}\nNotice: {}\n",
                summary.icon_count,
                summary.provider_name,
                output.display(),
                summary.manifest_path.display(),
                summary.notice_path.display()
            ),
            stdout,
            stderr,
        ),
        Err(error) => write_stderr_error(&format!("cannot import provider icons: {error}"), stderr),
    }
}

fn import_provider_from_official_sources(
    provider: &str,
    output: &Path,
) -> Result<provider::ImportSummary, provider::ImportError> {
    let mut download = download_provider_archive;
    provider::import_provider_pack_from_official_sources(provider, output, &mut download)
}

fn download_provider_archive(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(120)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let response = agent.get(url).call();
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            return Err(format!(
                "cannot download audited archive from '{url}': {error}"
            ));
        }
    };
    let body = response.body_mut().with_config().limit(limit).read_to_vec();
    match body {
        Ok(bytes) => Ok(bytes),
        Err(error) => Err(format!(
            "cannot download audited archive from '{url}': {error}"
        )),
    }
}

fn run_render(
    mut arguments: impl Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(source) = arguments.next() else {
        return argument_error("missing file for 'stack render'", stderr);
    };
    if source == OsStr::new("--help") || source == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(RENDER_HELP, stdout, stderr);
    }
    if source.to_string_lossy().starts_with('-') {
        return argument_error(
            &format!("unknown option '{}'", source.to_string_lossy()),
            stderr,
        );
    }

    let mut destination = None;
    let mut notice_path = None;
    let mut provider_pack_root = None;
    while let Some(option) = arguments.next() {
        if option == OsStr::new("-o") {
            if destination.is_some() {
                return argument_error("duplicate '-o' option", stderr);
            }
            let Some(output) = arguments.next() else {
                return argument_error("missing output file after '-o'", stderr);
            };
            destination = Some(RenderDestination::File(PathBuf::from(output)));
        } else if option == OsStr::new("--notice") {
            if notice_path.is_some() {
                return argument_error("duplicate '--notice' option", stderr);
            }
            let Some(path) = arguments.next() else {
                return argument_error("missing notice file after '--notice'", stderr);
            };
            notice_path = Some(PathBuf::from(path));
        } else if option == OsStr::new("--provider-pack") {
            if provider_pack_root.is_some() {
                return argument_error("duplicate '--provider-pack' option", stderr);
            }
            let Some(path) = arguments.next() else {
                return argument_error("missing provider icon-store directory", stderr);
            };
            provider_pack_root = Some(PathBuf::from(path));
        } else {
            return argument_error(
                &format!("unexpected argument '{}'", option.to_string_lossy()),
                stderr,
            );
        }
    }

    let destination = destination.unwrap_or(RenderDestination::Stdout);
    if matches!(&destination, RenderDestination::File(path) if path.as_os_str() == source) {
        return argument_error("input and output files must be different", stderr);
    }
    if notice_path
        .as_ref()
        .is_some_and(|path| path.as_os_str() == source)
    {
        return argument_error("input and notice files must be different", stderr);
    }
    if matches!(&destination, RenderDestination::File(path) if notice_path.as_ref() == Some(path)) {
        return argument_error("output and notice files must be different", stderr);
    }

    let explicit_provider_pack_root = provider_pack_root.is_some();
    let environment = config::Environment::capture();
    let provider_pack_root =
        match config::icon_store_root(provider_pack_root.as_deref(), &environment) {
            Ok(path) => path,
            Err(error) => return write_stderr_error(&error, stderr),
        };

    render_file(
        Path::new(&source),
        destination,
        &provider_pack_root,
        !explicit_provider_pack_root,
        notice_path.as_deref(),
        stdout,
        stderr,
    )
}

fn run_format(
    mut arguments: impl Iterator<Item = OsString>,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(first) = arguments.next() else {
        return argument_error("missing file for 'stack fmt'", stderr);
    };
    if first == OsStr::new("--help") || first == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(FORMAT_HELP, stdout, stderr);
    }

    let (mode, input) = if first == OsStr::new("--check") {
        let Some(input) = arguments.next() else {
            return argument_error("missing file for 'stack fmt --check'", stderr);
        };
        (FormatMode::Check, input)
    } else {
        (FormatMode::Write, first)
    };
    if input != OsStr::new("-") && input.to_string_lossy().starts_with('-') {
        return argument_error(
            &format!("unknown option '{}'", input.to_string_lossy()),
            stderr,
        );
    }
    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }

    if input == OsStr::new("-") {
        format_stdin(mode, stdin, stdout, stderr)
    } else {
        format_file(mode, Path::new(&input), stderr)
    }
}

fn run_check(
    mut arguments: impl Iterator<Item = OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let Some(path) = arguments.next() else {
        return argument_error("missing file for 'stack check'", stderr);
    };

    if path == OsStr::new("--help") || path == OsStr::new("-h") {
        if let Some(extra) = arguments.next() {
            return argument_error(
                &format!("unexpected argument '{}'", extra.to_string_lossy()),
                stderr,
            );
        }
        return write_stdout(CHECK_HELP, stdout, stderr);
    }

    if let Some(extra) = arguments.next() {
        return argument_error(
            &format!("unexpected argument '{}'", extra.to_string_lossy()),
            stderr,
        );
    }

    check_file(Path::new(&path), stderr)
}

fn check_file(path: &Path, stderr: &mut dyn Write) -> u8 {
    check_file_with(path, stderr, |source| Engine::bundled().check(source))
}

fn check_file_with(
    path: &Path,
    stderr: &mut dyn Write,
    check: impl FnOnce(&[u8]) -> Result<CheckOutput, OperationalError>,
) -> u8 {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) => {
            return write_stderr_error(
                &format!(
                    "cannot read '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    };

    let output = match check(&source) {
        Ok(output) => output,
        Err(error) => {
            return write_stderr_error(
                &format!("cannot check '{}': {error}", path.display()),
                stderr,
            );
        }
    };
    let has_errors = match write_diagnostics(path, &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };

    if has_errors {
        EXIT_STACK_ERROR
    } else {
        EXIT_SUCCESS
    }
}

fn render_file(
    path: &Path,
    destination: RenderDestination,
    provider_pack_root: &Path,
    allow_missing_provider_pack_root: bool,
    notice_path: Option<&Path>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let provider_packs =
        match load_provider_store(provider_pack_root, allow_missing_provider_pack_root) {
            Ok(provider_packs) => provider_packs,
            Err(reason) => {
                return write_stderr_error(
                    &format!(
                        "cannot load provider icon store '{}': {reason}",
                        provider_pack_root.display()
                    ),
                    stderr,
                );
            }
        };
    let engine = match Engine::with_provider_packs(&provider_packs) {
        Ok(engine) => engine,
        Err(error) => {
            return write_stderr_error(&format!("cannot load provider packs: {error}"), stderr);
        }
    };
    render_file_with(
        path,
        destination,
        notice_path,
        stdout,
        stderr,
        |source| engine.render(source),
        atomic_write_output,
    )
}

fn load_provider_store(root: &Path, allow_missing: bool) -> Result<Vec<ProviderPack>, String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if allow_missing && error.kind() == io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(stable_io_error(error.kind()).to_owned()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("store path must be a real directory, not a symlink".to_owned());
    }

    let mut packs = Vec::new();
    for provider_id in provider_catalog::PROVIDER_IDS {
        let pack_root = root.join(provider_id);
        match fs::symlink_metadata(&pack_root) {
            Ok(_) => packs.push(load_provider_pack_for(&pack_root, provider_id)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect '{}': {}",
                    pack_root.display(),
                    stable_io_error(error.kind())
                ));
            }
        }
    }
    Ok(packs)
}

fn render_file_with(
    path: &Path,
    destination: RenderDestination,
    notice_path: Option<&Path>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    render: impl FnOnce(&[u8]) -> Result<RenderOutput, OperationalError>,
    mut write_output: impl FnMut(&Path, &[u8]) -> io::Result<()>,
) -> u8 {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) => {
            return write_stderr_error(
                &format!(
                    "cannot read '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    };
    let output = match render(&source) {
        Ok(output) => output,
        Err(error) => {
            return write_stderr_error(
                &format!("cannot render '{}': {error}", path.display()),
                stderr,
            );
        }
    };
    let has_errors = match write_diagnostics(path, &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };
    if has_errors {
        return EXIT_STACK_ERROR;
    }
    let Some(svg) = output.svg else {
        return write_stderr_error("renderer produced no SVG or error diagnostic", stderr);
    };

    match destination {
        RenderDestination::Stdout => {
            if stdout.write_all(svg.as_bytes()).is_err() {
                return write_stderr_error("cannot write rendered SVG", stderr);
            }
        }
        RenderDestination::File(output_path) => {
            if let Err(error) = write_output(&output_path, svg.as_bytes()) {
                return write_stderr_error(
                    &format!(
                        "cannot write '{}': {}",
                        output_path.display(),
                        stable_io_error(error.kind())
                    ),
                    stderr,
                );
            }
        }
    }
    if let Some(notice_path) = notice_path {
        let notice = render_provider_notices(&output.provider_notices);
        if let Err(error) = write_output(notice_path, notice.as_bytes()) {
            return write_stderr_error(
                &format!(
                    "cannot write '{}': {}",
                    notice_path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    }
    EXIT_SUCCESS
}

#[cfg(test)]
fn load_provider_pack(root: &Path) -> Result<ProviderPack, String> {
    load_provider_pack_with_expected_id(root, None)
}

fn load_provider_pack_for(root: &Path, expected_provider_id: &str) -> Result<ProviderPack, String> {
    load_provider_pack_with_expected_id(root, Some(expected_provider_id))
}

fn load_provider_pack_with_expected_id(
    root: &Path,
    expected_provider_id: Option<&str>,
) -> Result<ProviderPack, String> {
    let root_metadata =
        fs::symlink_metadata(root).map_err(|error| stable_io_error(error.kind()).to_owned())?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("pack path must be a real directory, not a symlink".to_owned());
    }

    let manifest_path = root.join("manifest.json");
    let manifest_bytes = read_bounded_regular_file(&manifest_path, MAX_PROVIDER_MANIFEST_BYTES)?;
    let manifest: stack_theme::ProviderPack = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| "manifest.json is invalid".to_owned())?;
    if let Some(expected_provider_id) = expected_provider_id {
        if manifest.provider.id != expected_provider_id {
            return Err(format!(
                "manifest provider '{}' does not match directory '{}'",
                manifest.provider.id, expected_provider_id
            ));
        }
    }
    let mut assets = Vec::with_capacity(manifest.icons.len());
    for icon in &manifest.icons {
        let relative = safe_provider_asset_path(&icon.asset.path)?;
        let path = root.join(relative);
        let svg = read_bounded_regular_file(&path, MAX_PROVIDER_ASSET_BYTES)?;
        let svg =
            String::from_utf8(svg).map_err(|_| format!("'{}' is not UTF-8 SVG", path.display()))?;
        assets.push(ProviderAsset::new(&icon.asset.path, svg));
    }
    ProviderPack::new(manifest, assets).map_err(|error| error.to_string())
}

fn read_bounded_regular_file(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot read '{}': {}",
            path.display(),
            stable_io_error(error.kind())
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "'{}' must be a regular file, not a symlink",
            path.display()
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(format!("'{}' exceeds the size limit", path.display()));
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut contents))
        .map_err(|error| {
            format!(
                "cannot read '{}': {}",
                path.display(),
                stable_io_error(error.kind())
            )
        })?;
    if contents.len() > limit {
        return Err(format!("'{}' exceeds the size limit", path.display()));
    }
    Ok(contents)
}

fn safe_provider_asset_path(value: &str) -> Result<&Path, String> {
    let path = Path::new(value);
    let components = path.components().collect::<Vec<_>>();
    if components.len() != 2
        || components[0] != Component::Normal(OsStr::new("assets"))
        || !matches!(components[1], Component::Normal(_))
        || path.extension() != Some(OsStr::new("svg"))
    {
        return Err(format!("unsafe provider asset path '{value}'"));
    }
    Ok(path)
}

fn render_provider_notices(notices: &[ProviderNotice]) -> String {
    let mut output = String::from("# Stack provider icon notices\n");
    if notices.is_empty() {
        output.push_str("\nNo provider icons were embedded in this artifact.\n");
        return output;
    }
    for notice in notices {
        let _ = write!(
            output,
            "\n## {} (`{}`)\n\n- Pack version: {}\n- Pack revision: `{}`\n\n{}\n\n{}\n\n{}\n\nSources:\n",
            notice_text(&notice.provider_name),
            notice.provider_id,
            notice_text(&notice.pack_version),
            notice.pack_revision,
            notice_text(&notice.attribution),
            notice_text(&notice.terms_summary),
            notice_text(&notice.non_endorsement),
        );
        for source in &notice.sources {
            let _ = writeln!(
                output,
                "- `{}`: release {}; archive `{}`; terms {}",
                source.id,
                notice_text(&source.release),
                source.archive_sha256,
                notice_text(&source.terms_url),
            );
        }
        output.push_str("\nUsed icons:\n");
        for icon in &notice.icons {
            let _ = write!(
                output,
                "- `{}`: {} (source `{}`)",
                icon.id,
                notice_text(&icon.product_name),
                notice_text(&icon.source_id),
            );
            if let Some(url) = &icon.brand_source_url {
                let _ = write!(output, "; brand source {}", notice_text(url));
            }
            if let Some(url) = &icon.brand_guidelines_url {
                let _ = write!(output, "; brand guidelines {}", notice_text(url));
            }
            output.push('\n');
        }
    }
    output
}

fn notice_text(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            '<' | '>' => ' ',
            _ if character.is_control() => ' ',
            _ => character,
        })
        .collect()
}

fn format_file(mode: FormatMode, path: &Path, stderr: &mut dyn Write) -> u8 {
    format_file_with(
        mode,
        path,
        stderr,
        |source| Engine::bundled().format(source),
        atomic_replace,
    )
}

fn format_file_with(
    mode: FormatMode,
    path: &Path,
    stderr: &mut dyn Write,
    format: impl FnOnce(&[u8]) -> Result<FormatOutput, OperationalError>,
    replace: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> u8 {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) => {
            return write_stderr_error(
                &format!(
                    "cannot read '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    };
    let output = match format(&source) {
        Ok(output) => output,
        Err(error) => {
            return write_stderr_error(
                &format!("cannot format '{}': {error}", path.display()),
                stderr,
            );
        }
    };
    let has_errors = match write_diagnostics(path, &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };
    let Some(formatted) = output.formatted_source else {
        return if has_errors {
            EXIT_STACK_ERROR
        } else {
            write_stderr_error("formatter produced no source or error diagnostic", stderr)
        };
    };
    let changed = formatted.as_bytes() != source;

    if mode == FormatMode::Check {
        return if changed || has_errors {
            EXIT_STACK_ERROR
        } else {
            EXIT_SUCCESS
        };
    }
    if changed {
        if let Err(error) = replace(path, formatted.as_bytes()) {
            return write_stderr_error(
                &format!(
                    "cannot replace '{}': {}",
                    path.display(),
                    stable_io_error(error.kind())
                ),
                stderr,
            );
        }
    }

    if has_errors {
        EXIT_STACK_ERROR
    } else {
        EXIT_SUCCESS
    }
}

fn format_stdin(
    mode: FormatMode,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let mut source = Vec::new();
    if stdin.read_to_end(&mut source).is_err() {
        return write_stderr_error("cannot read standard input", stderr);
    }
    format_stdin_with(mode, &source, stdout, stderr, |source| {
        Engine::bundled().format(source)
    })
}

fn format_stdin_with(
    mode: FormatMode,
    source: &[u8],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    format: impl FnOnce(&[u8]) -> Result<FormatOutput, OperationalError>,
) -> u8 {
    let output = match format(source) {
        Ok(output) => output,
        Err(error) => return write_stderr_error(&format!("cannot format stdin: {error}"), stderr),
    };
    let has_errors = match write_diagnostics(Path::new("<stdin>"), &output.diagnostics, stderr) {
        Ok(has_errors) => has_errors,
        Err(()) => return EXIT_USAGE_OR_IO,
    };
    let Some(formatted) = output.formatted_source else {
        return if has_errors {
            EXIT_STACK_ERROR
        } else {
            write_stderr_error("formatter produced no source or error diagnostic", stderr)
        };
    };
    let changed = formatted.as_bytes() != source;

    if mode == FormatMode::Check {
        return if changed || has_errors {
            EXIT_STACK_ERROR
        } else {
            EXIT_SUCCESS
        };
    }
    if stdout.write_all(formatted.as_bytes()).is_err() {
        return write_stderr_error("cannot write formatted source", stderr);
    }
    if has_errors {
        EXIT_STACK_ERROR
    } else {
        EXIT_SUCCESS
    }
}

fn atomic_replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let permissions = fs::metadata(path)?.permissions();
    atomic_write(path, contents, Some(permissions))
}

fn create_new_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

fn atomic_write_output(path: &Path, contents: &[u8]) -> io::Result<()> {
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    atomic_write(path, contents, permissions)
}

fn atomic_write(
    path: &Path,
    contents: &[u8],
    permissions: Option<fs::Permissions>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (temporary_path, mut temporary_file) = create_temporary_file(parent)?;

    let prepared = temporary_file
        .write_all(contents)
        .and_then(|()| match permissions {
            Some(permissions) => temporary_file.set_permissions(permissions),
            None => Ok(()),
        })
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);
    if let Err(error) = prepared {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(())
}

fn create_temporary_file(parent: &Path) -> io::Result<(PathBuf, File)> {
    for attempt in 0..128_u8 {
        let path = parent.join(format!(".stack-tmp-{}-{attempt}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve an atomic replacement file",
    ))
}

fn write_diagnostics(
    path: &Path,
    diagnostics: &[Diagnostic],
    stderr: &mut dyn Write,
) -> Result<bool, ()> {
    let has_errors = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    let rendered = render_diagnostics(path, diagnostics);
    if !rendered.is_empty() && stderr.write_all(rendered.as_bytes()).is_err() {
        return Err(());
    }
    Ok(has_errors)
}

fn render_diagnostics(path: &Path, diagnostics: &[Diagnostic]) -> String {
    let mut rendered = String::new();
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let _ = writeln!(
            rendered,
            "{}:{}:{}: {severity}[{}]: {}",
            path.display(),
            diagnostic.range.start.line,
            diagnostic.range.start.column,
            diagnostic.code,
            diagnostic.message
        );
        if !diagnostic.expected.is_empty() {
            let _ = writeln!(rendered, "  expected: {}", diagnostic.expected.join(", "));
        }
        if let Some(help) = &diagnostic.help {
            let _ = writeln!(rendered, "  help: {help}");
        }
        for related in &diagnostic.related {
            let _ = writeln!(
                rendered,
                "{}:{}:{}: note: {}",
                path.display(),
                related.range.start.line,
                related.range.start.column,
                related.message
            );
        }
    }
    rendered
}

fn stable_io_error(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "file not found",
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::InvalidData => "invalid data",
        _ => "I/O error",
    }
}

fn is_help_flag(value: &OsStr) -> bool {
    value == OsStr::new("--help") || value == OsStr::new("-h")
}

fn is_version_flag(value: &OsStr) -> bool {
    value == OsStr::new("--version") || value == OsStr::new("-V") || value == OsStr::new("-v")
}

fn write_version(stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    write_stdout(
        concat!("stack ", env!("CARGO_PKG_VERSION"), "\n"),
        stdout,
        stderr,
    )
}

fn unknown_command_error(
    scope: &str,
    command: &OsStr,
    candidates: &[&str],
    help_command: &str,
    stderr: &mut dyn Write,
) -> u8 {
    let command = command.to_string_lossy();
    if scope == "stack" {
        let _ = writeln!(stderr, "error: unknown command '{command}'");
    } else {
        let _ = writeln!(stderr, "error: unknown command for '{scope}': '{command}'");
    }
    if let Some(suggestion) = command_suggestion(&command, candidates) {
        let _ = writeln!(stderr, "\nDid you mean '{suggestion}'?");
    }
    let _ = writeln!(stderr, "\nFor more information, try '{help_command}'.");
    EXIT_USAGE_OR_IO
}

fn command_suggestion<'a>(input: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let input_length = input.chars().count();
    candidates
        .iter()
        .map(|candidate| (*candidate, edit_distance(input, candidate)))
        .filter(|(candidate, distance)| {
            *distance <= 2 && *distance * 2 <= input_length.max(candidate.chars().count())
        })
        .min_by_key(|(_, distance)| *distance)
        .map(|(candidate, _)| candidate)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_character) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != *right_character);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current.push(substitution.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}

fn argument_error(message: &str, stderr: &mut dyn Write) -> u8 {
    let _ = writeln!(stderr, "error: {message}\n\n{GENERAL_HELP}");
    EXIT_USAGE_OR_IO
}

fn write_stderr_error(message: &str, stderr: &mut dyn Write) -> u8 {
    let _ = writeln!(stderr, "error: {message}");
    EXIT_USAGE_OR_IO
}

fn write_stdout(message: &str, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    if stdout.write_all(message.as_bytes()).is_ok() {
        EXIT_SUCCESS
    } else {
        write_stderr_error("cannot write command output", stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingWriter;

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("test reader failure"))
        }
    }

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("test writer failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn run_without_input(
        arguments: impl IntoIterator<Item = OsString>,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
    ) -> u8 {
        run(arguments, &mut io::empty(), stdout, stderr)
    }

    #[test]
    fn help_version_and_argument_errors_have_stable_streams() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_without_input([OsString::from("--version")], &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, b"stack 0.3.0\n");
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(
            run_without_input([OsString::from("--help")], &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, GENERAL_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input([], &mut stdout, &mut stderr),
            EXIT_USAGE_OR_IO
        );
        assert!(stdout.is_empty());
        assert!(String::from_utf8_lossy(&stderr).contains("error: missing command"));
    }

    #[test]
    fn command_aliases_and_invalid_arguments_are_stable() {
        for alias in ["-h", "-V"] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_without_input([OsString::from(alias)], &mut stdout, &mut stderr),
                EXIT_SUCCESS
            );
            assert!(!stdout.is_empty());
            assert!(stderr.is_empty());
        }

        for arguments in [
            vec![OsString::from("unknown")],
            vec![OsString::from("--help"), OsString::from("extra")],
            vec![OsString::from("--version"), OsString::from("extra")],
            vec![
                OsString::from("init"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("init"), OsString::from("--template")],
            vec![
                OsString::from("init"),
                OsString::from("--template"),
                OsString::from("--force"),
            ],
            vec![
                OsString::from("init"),
                OsString::from("--template"),
                OsString::from("hello-stack"),
                OsString::from("--template"),
                OsString::from("application-and-data"),
            ],
            vec![OsString::from("init"), OsString::from("-o")],
            vec![
                OsString::from("init"),
                OsString::from("--output"),
                OsString::from("--force"),
            ],
            vec![
                OsString::from("init"),
                OsString::from("-o"),
                OsString::from("one.stack"),
                OsString::from("--output"),
                OsString::from("two.stack"),
            ],
            vec![
                OsString::from("init"),
                OsString::from("--force"),
                OsString::from("--force"),
            ],
            vec![OsString::from("init"), OsString::from("--unknown")],
            vec![OsString::from("init"), OsString::from("unexpected")],
            vec![
                OsString::from("init"),
                OsString::from("--template"),
                OsString::from("unknown"),
            ],
            vec![OsString::from("check")],
            vec![
                OsString::from("check"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("fmt")],
            vec![
                OsString::from("fmt"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("fmt"), OsString::from("--check")],
            vec![OsString::from("fmt"), OsString::from("--unknown")],
            vec![
                OsString::from("fmt"),
                OsString::from("file.stack"),
                OsString::from("extra"),
            ],
            vec![OsString::from("render")],
            vec![
                OsString::from("render"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("render"), OsString::from("--unknown")],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("unexpected"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("out.svg"),
                OsString::from("extra"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("one.svg"),
                OsString::from("-o"),
                OsString::from("two.svg"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--notice"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--notice"),
                OsString::from("one.md"),
                OsString::from("--notice"),
                OsString::from("two.md"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--provider-pack"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("file.stack"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("--notice"),
                OsString::from("file.stack"),
            ],
            vec![
                OsString::from("render"),
                OsString::from("file.stack"),
                OsString::from("-o"),
                OsString::from("artifact"),
                OsString::from("--notice"),
                OsString::from("artifact"),
            ],
            vec![OsString::from("icons")],
            vec![OsString::from("icons"), OsString::from("unknown")],
            vec![
                OsString::from("icons"),
                OsString::from("list"),
                OsString::from("aws"),
                OsString::from("s3"),
                OsString::from("extra"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("--help"),
                OsString::from("extra"),
            ],
            vec![OsString::from("icons"), OsString::from("import")],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("aws"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("aws"),
                OsString::from("icons.zip"),
                OsString::from("-o"),
                OsString::from("pack"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("aws"),
                OsString::from("icons.zip"),
                OsString::from("--accept-terms"),
                OsString::from("-o"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("--unknown"),
            ],
            vec![
                OsString::from("icons"),
                OsString::from("import"),
                OsString::from("gcp"),
                OsString::from("core.zip"),
                OsString::from("--source"),
                OsString::from("invalid"),
                OsString::from("--accept-terms"),
                OsString::from("-o"),
                OsString::from("pack"),
            ],
            vec![
                OsString::from("check"),
                OsString::from("file.stack"),
                OsString::from("extra"),
            ],
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            assert_eq!(
                run_without_input(arguments, &mut stdout, &mut stderr),
                EXIT_USAGE_OR_IO
            );
            assert!(stdout.is_empty());
            assert!(String::from_utf8_lossy(&stderr).starts_with("error:"));
        }

        let duplicate_provider_store = vec![
            OsString::from("render"),
            OsString::from("file.stack"),
            OsString::from("--provider-pack"),
            OsString::from("first"),
            OsString::from("--provider-pack"),
            OsString::from("second"),
        ];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_without_input(duplicate_provider_store, &mut stdout, &mut stderr),
            EXIT_USAGE_OR_IO
        );
        assert!(String::from_utf8_lossy(&stderr).contains("duplicate '--provider-pack'"));

        stdout.clear();
        stderr.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("check"), OsString::from("-h")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, CHECK_HELP.as_bytes());
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("fmt"), OsString::from("-h")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, FORMAT_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("render"), OsString::from("--help")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, RENDER_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [OsString::from("icons"), OsString::from("--help")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, ICONS_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [
                    OsString::from("icons"),
                    OsString::from("import"),
                    OsString::from("--help"),
                ],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, ICONS_IMPORT_HELP.as_bytes());

        stdout.clear();
        assert_eq!(
            run_without_input(
                [
                    OsString::from("icons"),
                    OsString::from("list"),
                    OsString::from("--help"),
                ],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, ICONS_LIST_HELP.as_bytes());
    }

    #[test]
    fn provider_catalog_listing_is_searchable_without_asset_bytes() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_without_input(
                [OsString::from("icons"), OsString::from("list")],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        let listing = String::from_utf8_lossy(&stdout);
        assert!(listing.contains("aws\t305"));
        assert!(listing.contains("azure\t639"));
        assert!(listing.contains("simple-icons\t62"));

        stdout.clear();
        assert_eq!(
            run_without_input(
                [
                    OsString::from("icons"),
                    OsString::from("list"),
                    OsString::from("aws"),
                    OsString::from("s3"),
                ],
                &mut stdout,
                &mut stderr,
            ),
            EXIT_SUCCESS
        );
        let listing = String::from_utf8_lossy(&stdout);
        assert!(listing.contains("aws:s3\tAmazon Simple Storage Service"));
        assert!(listing.lines().count() >= 2);
        assert!(stderr.is_empty());
    }

    #[test]
    fn provider_import_command_resolves_a_store_root_and_reports_results() {
        fn successful_import(
            provider_id: &str,
            output: &Path,
        ) -> Result<provider::ImportSummary, provider::ImportError> {
            assert_eq!(provider_id, "gcp");
            assert_eq!(output, Path::new(".stack-icons/gcp"));
            Ok(provider::ImportSummary {
                provider_name: "Google Cloud".to_owned(),
                icon_count: 45,
                manifest_path: output.join("manifest.json"),
                notice_path: output.join("NOTICE.md"),
            })
        }

        fn failed_import(
            _: &str,
            _: &Path,
        ) -> Result<provider::ImportSummary, provider::ImportError> {
            Err(provider::ImportError::new("download failed"))
        }

        let mut arguments = [
            OsString::from("gcp"),
            OsString::from("--accept-terms"),
            OsString::from("-o"),
            OsString::from(".stack-icons"),
        ]
        .into_iter();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status =
            run_icons_import_with(&mut arguments, &mut stdout, &mut stderr, successful_import);
        assert_eq!(status, EXIT_SUCCESS);
        assert!(stderr.is_empty());
        let rendered_stdout = String::from_utf8_lossy(&stdout);
        assert!(rendered_stdout.contains("Imported 45 Google Cloud icons"));
        assert!(rendered_stdout.contains(".stack-icons/gcp/manifest.json"));

        let mut arguments = [
            OsString::from("aws"),
            OsString::from("--accept-terms"),
            OsString::from("-o"),
            OsString::from(".stack-icons"),
        ]
        .into_iter();
        stdout.clear();
        stderr.clear();
        let status = run_icons_import_with(&mut arguments, &mut stdout, &mut stderr, failed_import);
        assert_eq!(status, EXIT_USAGE_OR_IO);
        assert!(stdout.is_empty());
        assert!(String::from_utf8_lossy(&stderr).contains("download failed"));

        assert!(import_provider_from_official_sources("unknown", Path::new("unused")).is_err());
        assert!(download_provider_archive("http://example.com/archive.zip", 1).is_err());
    }

    #[test]
    fn diagnostic_rendering_includes_related_locations() {
        let diagnostic = Diagnostic {
            code: "STKTEST".to_owned(),
            severity: Severity::Error,
            message: "primary".to_owned(),
            range: stack_engine::SourceRange {
                start: stack_engine::SourcePosition {
                    byte_offset: 0,
                    line: 2,
                    column: 3,
                },
                end: stack_engine::SourcePosition {
                    byte_offset: 1,
                    line: 2,
                    column: 4,
                },
            },
            expected: vec!["node".to_owned(), "group".to_owned()],
            help: Some("choose a declaration".to_owned()),
            related: vec![stack_engine::RelatedInformation {
                message: "related".to_owned(),
                range: stack_engine::SourceRange {
                    start: stack_engine::SourcePosition {
                        byte_offset: 2,
                        line: 4,
                        column: 5,
                    },
                    end: stack_engine::SourcePosition {
                        byte_offset: 3,
                        line: 4,
                        column: 6,
                    },
                },
            }],
        };

        assert_eq!(
            render_diagnostics(Path::new("arch.stack"), &[diagnostic]),
            "arch.stack:2:3: error[STKTEST]: primary\n  expected: node, group\n  help: choose a declaration\narch.stack:4:5: note: related\n"
        );
    }

    #[test]
    fn host_error_labels_and_stream_failures_are_stable() {
        assert_eq!(
            stable_io_error(io::ErrorKind::PermissionDenied),
            "permission denied"
        );
        assert_eq!(stable_io_error(io::ErrorKind::InvalidData), "invalid data");
        assert_eq!(stable_io_error(io::ErrorKind::Other), "I/O error");

        let mut failed_stdout = FailingWriter;
        let mut stderr = Vec::new();
        assert_eq!(
            write_stdout("help", &mut failed_stdout, &mut stderr),
            EXIT_USAGE_OR_IO
        );
        assert_eq!(stderr, b"error: cannot write command output\n");
        assert!(failed_stdout.flush().is_ok());

        let path = std::env::temp_dir().join(format!(
            "stack-cli-writer-failure-{}.stack",
            std::process::id()
        ));
        assert!(
            fs::write(
                &path,
                b"stack 1.0 diagram \"Fallback\" { theme neon node api \"API\" }",
            )
            .is_ok()
        );
        let mut failed_stderr = FailingWriter;
        let status = check_file(&path, &mut failed_stderr);
        let mut operational_stderr = Vec::new();
        let operational_status = check_file_with(&path, &mut operational_stderr, |_| {
            Err(OperationalError::InvalidIntermediateRepresentation {
                reason: "test failure",
            })
        });
        assert!(fs::remove_file(path).is_ok());
        assert_eq!(status, EXIT_USAGE_OR_IO);
        assert_eq!(operational_status, EXIT_USAGE_OR_IO);
        assert!(String::from_utf8_lossy(&operational_stderr).contains("error: cannot check"));
    }

    #[test]
    fn format_stream_failures_use_host_exit_status() {
        let source = b"stack 1.0 diagram \"Valid\" { node api \"API\" }";
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(
                [OsString::from("fmt"), OsString::from("-")],
                &mut FailingReader,
                &mut stdout,
                &mut stderr,
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(String::from_utf8_lossy(&stderr).contains("cannot read standard input"));

        stderr.clear();
        assert_eq!(
            format_stdin_with(FormatMode::Write, source, &mut stdout, &mut stderr, |_| {
                Err(OperationalError::InvalidIntermediateRepresentation {
                    reason: "test failure",
                })
            },),
            EXIT_USAGE_OR_IO
        );
        assert!(String::from_utf8_lossy(&stderr).contains("cannot format stdin"));

        let output = Engine::bundled().format(source);
        assert!(output.is_ok());
        let Ok(mut empty_output) = output else {
            return;
        };
        empty_output.formatted_source = None;
        empty_output.diagnostics.clear();
        stderr.clear();
        assert_eq!(
            format_stdin_with(FormatMode::Write, source, &mut stdout, &mut stderr, |_| Ok(
                empty_output
            ),),
            EXIT_USAGE_OR_IO
        );

        let syntax = b"stack 1.0 diagram \"Incomplete\" {";
        let mut failed_stderr = FailingWriter;
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                syntax,
                &mut stdout,
                &mut failed_stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_USAGE_OR_IO
        );

        stderr.clear();
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                syntax,
                &mut stdout,
                &mut stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_STACK_ERROR
        );

        let semantic = b"stack 1.0 diagram \"Invalid\" { node api \"A\" node api \"B\" }";
        stdout.clear();
        stderr.clear();
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                semantic,
                &mut stdout,
                &mut stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_STACK_ERROR
        );
        assert!(!stdout.is_empty());

        let mut failed_stdout = FailingWriter;
        stderr.clear();
        assert_eq!(
            format_stdin_with(
                FormatMode::Write,
                source,
                &mut failed_stdout,
                &mut stderr,
                |source| Engine::bundled().format(source),
            ),
            EXIT_USAGE_OR_IO
        );
    }

    #[test]
    fn format_file_failures_do_not_replace_the_source() {
        let path = std::env::temp_dir().join(format!(
            "stack-cli-format-failures-{}.stack",
            std::process::id()
        ));
        let source = b"stack 1.0 diagram \"Valid\"{node api \"API\"}";
        assert!(fs::write(&path, source).is_ok());

        let mut stderr = Vec::new();
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut stderr,
                |_| {
                    Err(OperationalError::InvalidIntermediateRepresentation {
                        reason: "test failure",
                    })
                },
                atomic_replace,
            ),
            EXIT_USAGE_OR_IO
        );

        let syntax = b"stack 1.0 diagram \"Incomplete\" {";
        assert!(fs::write(&path, syntax).is_ok());
        let mut failed_stderr = FailingWriter;
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut failed_stderr,
                |source| Engine::bundled().format(source),
                atomic_replace,
            ),
            EXIT_USAGE_OR_IO
        );
        assert_eq!(fs::read(&path).ok().as_deref(), Some(syntax.as_slice()));
        assert!(fs::write(&path, source).is_ok());

        let output = Engine::bundled().format(source);
        assert!(output.is_ok());
        let Ok(mut empty_output) = output else {
            return;
        };
        empty_output.formatted_source = None;
        empty_output.diagnostics.clear();
        stderr.clear();
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut stderr,
                |_| Ok(empty_output),
                atomic_replace,
            ),
            EXIT_USAGE_OR_IO
        );

        stderr.clear();
        assert_eq!(
            format_file_with(
                FormatMode::Write,
                &path,
                &mut stderr,
                |source| Engine::bundled().format(source),
                |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            ),
            EXIT_USAGE_OR_IO
        );
        assert_eq!(fs::read(&path).ok().as_deref(), Some(source.as_slice()));
        assert!(fs::remove_file(path).is_ok());
    }

    #[test]
    fn atomic_replace_removes_its_temporary_file_after_rename_failure() {
        let root =
            std::env::temp_dir().join(format!("stack-cli-rename-failure-{}", std::process::id()));
        let target = root.join("target");
        assert!(fs::create_dir(&root).is_ok());
        assert!(fs::create_dir(&target).is_ok());
        assert!(fs::write(target.join("keep"), b"keep").is_ok());

        assert!(atomic_replace(&target, b"formatted").is_err());
        assert_eq!(
            fs::read_dir(&root).ok().map(|entries| entries.count()),
            Some(1)
        );
        assert!(target.join("keep").is_file());
        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn temporary_file_creation_skips_collisions_and_is_bounded() {
        let parent =
            std::env::temp_dir().join(format!("stack-cli-temporary-files-{}", std::process::id()));
        assert!(fs::create_dir(&parent).is_ok());
        for attempt in 0..128_u8 {
            assert!(
                fs::write(
                    parent.join(format!(".stack-tmp-{}-{attempt}", std::process::id())),
                    b"collision",
                )
                .is_ok()
            );
        }
        let exhausted = create_temporary_file(&parent);
        assert_eq!(
            exhausted.err().map(|error| error.kind()),
            Some(io::ErrorKind::AlreadyExists)
        );
        assert!(fs::remove_dir_all(parent).is_ok());

        let missing =
            std::env::temp_dir().join(format!("stack-cli-missing-parent-{}", std::process::id()));
        assert_eq!(
            create_temporary_file(&missing)
                .err()
                .map(|error| error.kind()),
            Some(io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn provider_pack_loader_rejects_untrusted_host_inputs() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("provider-pack");
        let pack = load_provider_pack(&fixture);
        assert_eq!(
            pack.ok().map(|pack| pack.manifest().provider.id.clone()),
            Some("example".to_owned())
        );

        assert!(safe_provider_asset_path("assets/icon.svg").is_ok());
        for path in [
            "icon.svg",
            "assets",
            "assets/nested/icon.svg",
            "assets/../icon.svg",
            "/assets/icon.svg",
            "assets/icon.png",
        ] {
            assert!(safe_provider_asset_path(path).is_err(), "accepted {path}");
        }

        let root =
            std::env::temp_dir().join(format!("stack-cli-provider-loader-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        assert!(load_provider_pack(&root).is_err());
        assert!(fs::write(&root, b"not a directory").is_ok());
        assert!(load_provider_pack(&root).is_err());
        assert!(fs::remove_file(&root).is_ok());
        assert!(fs::create_dir_all(root.join("assets")).is_ok());

        let manifest_path = root.join("manifest.json");
        assert!(fs::write(&manifest_path, b"{}").is_ok());
        assert!(load_provider_pack(&root).is_err());
        assert!(
            fs::write(
                &manifest_path,
                include_bytes!("../tests/fixtures/provider-pack/manifest.json")
            )
            .is_ok()
        );
        let asset_path = root.join("assets/storage.svg");
        assert!(fs::write(&asset_path, [0xff]).is_ok());
        assert!(load_provider_pack(&root).is_err());
        assert!(fs::write(&asset_path, b"<svg/>").is_ok());
        assert!(load_provider_pack(&root).is_err());

        let oversized = root.join("oversized");
        assert!(fs::write(&oversized, vec![b'x'; 5]).is_ok());
        assert!(read_bounded_regular_file(&oversized, 4).is_err());
        assert!(read_bounded_regular_file(&root, 4).is_err());
        assert_eq!(
            read_bounded_regular_file(&oversized, 5).ok(),
            Some(vec![b'x'; 5])
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let linked = root.join("linked");
            assert!(symlink(&oversized, &linked).is_ok());
            assert!(read_bounded_regular_file(&linked, 5).is_err());
            assert!(load_provider_pack(&linked).is_err());
        }
        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn provider_store_discovers_only_known_provider_directories() {
        let root =
            std::env::temp_dir().join(format!("stack-cli-provider-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        assert!(load_provider_store(&root, true).is_ok_and(|packs| packs.is_empty()));
        assert!(fs::create_dir_all(root.join("unknown")).is_ok());
        assert!(fs::write(root.join("unknown/manifest.json"), b"invalid").is_ok());
        assert!(load_provider_store(&root, false).is_ok_and(|packs| packs.is_empty()));

        assert!(fs::create_dir_all(root.join("aws")).is_ok());
        assert!(fs::write(root.join("aws/manifest.json"), b"invalid").is_ok());
        assert!(load_provider_store(&root, false).is_err());
        assert!(fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn provider_notice_text_is_stable_and_inert() {
        assert_eq!(
            render_provider_notices(&[]),
            "# Stack provider icon notices\n\nNo provider icons were embedded in this artifact.\n"
        );
        assert_eq!(notice_text("line\n<unsafe>\ttext"), "line  unsafe  text");
    }

    #[test]
    fn render_failures_do_not_emit_partial_artifacts() {
        let path = std::env::temp_dir().join(format!(
            "stack-cli-render-failures-{}.stack",
            std::process::id()
        ));
        let source = b"stack 1.0 diagram \"Valid\" { node api \"API\" }";
        assert!(fs::write(&path, source).is_ok());

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut stdout,
                &mut stderr,
                |_| {
                    Err(OperationalError::InvalidIntermediateRepresentation {
                        reason: "test failure",
                    })
                },
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(stdout.is_empty());

        let output = Engine::bundled().render(source);
        assert!(output.is_ok());
        let Ok(mut empty_output) = output else {
            return;
        };
        empty_output.svg = None;
        empty_output.diagnostics.clear();
        stderr.clear();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut stdout,
                &mut stderr,
                |_| Ok(empty_output),
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );

        let mut failed_stdout = FailingWriter;
        stderr.clear();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut failed_stdout,
                &mut stderr,
                |source| Engine::bundled().render(source),
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );

        let output_path = path.with_extension("svg");
        stderr.clear();
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::File(output_path.clone()),
                None,
                &mut stdout,
                &mut stderr,
                |source| Engine::bundled().render(source),
                |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(!output_path.exists());

        let syntax = b"stack 1.0 diagram \"Incomplete\" {";
        assert!(fs::write(&path, syntax).is_ok());
        let mut failed_stderr = FailingWriter;
        assert_eq!(
            render_file_with(
                &path,
                RenderDestination::Stdout,
                None,
                &mut stdout,
                &mut failed_stderr,
                |source| Engine::bundled().render(source),
                atomic_write_output,
            ),
            EXIT_USAGE_OR_IO
        );
        assert!(stdout.is_empty());
        assert!(fs::remove_file(path).is_ok());
    }
}
