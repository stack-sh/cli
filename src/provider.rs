//! Local-only provider icon archive import.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use roxmltree::{Document, Node, NodeType};
use sha2::{Digest, Sha256};
use stack_theme::{
    ProviderIcon, ProviderIconAsset, ProviderNodeKind, ProviderPack, ProviderPackDistributionMode,
    ProviderPackIdentity, ProviderPackModificationPolicy, ProviderPackNotice,
    ProviderPackPermittedOutput, ProviderPackProcessing, ProviderPackRedistribution,
    ProviderPackRights, ProviderPackSource, ProviderPackTransformation,
};
use zip::ZipArchive;

const PROVIDER_PACK_SCHEMA: &str =
    "https://raw.githubusercontent.com/stack-sh/theme/main/schemas/provider-pack.schema.json";
const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ICON_BYTES: u64 = 1024 * 1024;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

const ALLOWED_ELEMENTS: &[&str] = &[
    "circle",
    "defs",
    "ellipse",
    "g",
    "line",
    "linearGradient",
    "path",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "svg",
];

const ALLOWED_ATTRIBUTES: &[&str] = &[
    "aria-hidden",
    "clip-rule",
    "cx",
    "cy",
    "d",
    "fill",
    "fill-rule",
    "gradientTransform",
    "gradientUnits",
    "height",
    "id",
    "opacity",
    "offset",
    "points",
    "r",
    "role",
    "rx",
    "ry",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-width",
    "transform",
    "viewBox",
    "width",
    "x",
    "x1",
    "x2",
    "y",
    "y1",
    "y2",
];

#[derive(Clone, Copy)]
struct IconProfile<'a> {
    slug: &'a str,
    subject: &'a str,
    product_name: &'a str,
    node_kind: ProviderNodeKind,
    archive_path: &'a str,
}

#[derive(Clone, Copy)]
struct ProviderProfile<'a> {
    id: &'a str,
    name: &'a str,
    page_url: &'a str,
    archive_url: &'a str,
    archive_sha256: &'a str,
    release: &'a str,
    retrieved_at: &'a str,
    terms_url: &'a str,
    terms_reviewed_at: &'a str,
    review_after: &'a str,
    copyright: &'a str,
    license_id: &'a str,
    archive_license_included: bool,
    permitted_outputs: &'a [ProviderPackPermittedOutput],
    product_name_nearby: bool,
    attribution: &'a str,
    terms_summary: &'a str,
    non_endorsement: &'a str,
    icons: &'a [IconProfile<'a>],
}

/// Summary of one completed local provider-pack import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportSummary {
    pub(crate) provider_name: String,
    pub(crate) icon_count: usize,
    pub(crate) manifest_path: PathBuf,
    pub(crate) notice_path: PathBuf,
}

/// Stable local import error without archive-library implementation details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportError {
    message: String,
}

impl ImportError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ImportError {}

struct ProcessedIcon {
    svg: String,
    view_box: [i32; 4],
    transformations: Vec<ProviderPackTransformation>,
}

/// Imports one audited official archive into a new local pack directory.
pub(crate) fn import_provider_pack(
    provider: &str,
    archive_path: &Path,
    output_path: &Path,
) -> Result<ImportSummary, ImportError> {
    let profile = match provider_profile(provider) {
        Some(profile) => profile,
        None => {
            return Err(ImportError::new(format!(
                "unknown provider '{provider}'; expected aws, gcp, or azure"
            )));
        }
    };
    import_profile(profile, archive_path, output_path)
}

fn import_profile(
    profile: ProviderProfile<'_>,
    archive_path: &Path,
    output_path: &Path,
) -> Result<ImportSummary, ImportError> {
    if output_path.exists() {
        return Err(ImportError::new(format!(
            "output '{}' already exists",
            output_path.display()
        )));
    }
    let metadata = match fs::metadata(archive_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(ImportError::new(format!(
                "cannot read archive '{}': {}",
                archive_path.display(),
                stable_io_error(error.kind())
            )));
        }
    };
    if !metadata.is_file() {
        return Err(ImportError::new(format!(
            "archive '{}' is not a file",
            archive_path.display()
        )));
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(ImportError::new(format!(
            "archive '{}' exceeds the 32 MiB limit",
            archive_path.display()
        )));
    }
    let archive_bytes = match fs::read(archive_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(ImportError::new(format!(
                "cannot read archive '{}': {}",
                archive_path.display(),
                stable_io_error(error.kind())
            )));
        }
    };
    let archive_digest = sha256(&archive_bytes);
    if archive_digest != profile.archive_sha256 {
        return Err(ImportError::new(format!(
            "archive hash does not match the audited {} release; expected {}, received {}",
            profile.release, profile.archive_sha256, archive_digest
        )));
    }

    let mut archive = match ZipArchive::new(Cursor::new(archive_bytes)) {
        Ok(archive) => archive,
        Err(_) => return Err(ImportError::new("archive is not a supported ZIP file")),
    };
    let mut manifest_icons = Vec::with_capacity(profile.icons.len());
    let mut processed_assets = Vec::with_capacity(profile.icons.len());

    for icon in profile.icons {
        let original = read_icon_entry(&mut archive, icon.archive_path)?;
        let original_sha256 = sha256(&original);
        let processed = sanitize_svg(&original, profile.id, icon.slug)?;
        let processed_sha256 = sha256(processed.svg.as_bytes());
        let asset_path = format!("assets/{}.svg", icon.slug);
        manifest_icons.push(ProviderIcon {
            id: format!("{}:{}", profile.id, icon.slug),
            subject: icon.subject.to_owned(),
            product_name: icon.product_name.to_owned(),
            recommended_node_kind: icon.node_kind,
            asset: ProviderIconAsset {
                path: asset_path.clone(),
                original_path: icon.archive_path.to_owned(),
                view_box: processed.view_box,
                original_sha256,
                processed_sha256,
                transformations: processed.transformations,
            },
        });
        processed_assets.push((asset_path, processed.svg));
    }

    let manifest = ProviderPack {
        schema: PROVIDER_PACK_SCHEMA.to_owned(),
        schema_version: "1.0".to_owned(),
        pack_version: "0.1.0".to_owned(),
        provider: ProviderPackIdentity {
            id: profile.id.to_owned(),
            name: profile.name.to_owned(),
        },
        distribution_mode: ProviderPackDistributionMode::UserImported,
        source: ProviderPackSource {
            page_url: profile.page_url.to_owned(),
            archive_url: profile.archive_url.to_owned(),
            archive_sha256: profile.archive_sha256.to_owned(),
            release: profile.release.to_owned(),
            retrieved_at: profile.retrieved_at.to_owned(),
            terms_url: profile.terms_url.to_owned(),
            terms_reviewed_at: profile.terms_reviewed_at.to_owned(),
            review_after: profile.review_after.to_owned(),
            copyright: profile.copyright.to_owned(),
            license_id: profile.license_id.to_owned(),
            archive_license_included: profile.archive_license_included,
        },
        rights: ProviderPackRights {
            terms_acceptance_required: true,
            permitted_outputs: profile.permitted_outputs.to_vec(),
            redistribution: ProviderPackRedistribution {
                cargo: false,
                npm: false,
                wasm: false,
                web_asset: false,
                native_binary: false,
                generated_output: true,
            },
            processing: ProviderPackProcessing {
                local_only: true,
                automatic_download: false,
                server_upload: false,
                preserve_colors: true,
                preserve_geometry: true,
                product_name_nearby: profile.product_name_nearby,
            },
            modification_policy: ProviderPackModificationPolicy::VisualPreservationOnly,
        },
        notice: ProviderPackNotice {
            attribution: profile.attribution.to_owned(),
            terms_summary: profile.terms_summary.to_owned(),
            non_endorsement: profile.non_endorsement.to_owned(),
        },
        icons: manifest_icons,
    };
    let mut manifest_bytes = match serde_json::to_vec_pretty(&manifest) {
        Ok(bytes) => bytes,
        Err(_) => return Err(ImportError::new("cannot serialize provider manifest")),
    };
    manifest_bytes.push(b'\n');
    let notice = render_notice(&manifest);

    let parent = match output_path.parent() {
        Some(path) if !path.as_os_str().is_empty() => path,
        _ => Path::new("."),
    };
    if !parent.is_dir() {
        return Err(ImportError::new(format!(
            "output parent '{}' is not a directory",
            parent.display()
        )));
    }
    let temporary_path = create_temporary_directory(parent)?;
    let write_result = write_pack_directory(
        &temporary_path,
        &manifest_bytes,
        notice.as_bytes(),
        &processed_assets,
    );
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&temporary_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary_path, output_path) {
        let _ = fs::remove_dir_all(&temporary_path);
        return Err(ImportError::new(format!(
            "cannot create output '{}': {}",
            output_path.display(),
            stable_io_error(error.kind())
        )));
    }

    Ok(ImportSummary {
        provider_name: profile.name.to_owned(),
        icon_count: manifest.icons.len(),
        manifest_path: output_path.join("manifest.json"),
        notice_path: output_path.join("NOTICE.md"),
    })
}

fn read_icon_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    entry_path: &str,
) -> Result<Vec<u8>, ImportError> {
    let mut entry = match archive.by_name(entry_path) {
        Ok(entry) => entry,
        Err(_) => {
            return Err(ImportError::new(format!(
                "archive is missing required icon '{entry_path}'"
            )));
        }
    };
    if entry.is_dir() || entry.is_symlink() {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' is not a regular file"
        )));
    }
    if entry.enclosed_name().as_deref() != Some(Path::new(entry_path)) {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' has an unsafe path"
        )));
    }
    if entry.size() > MAX_ICON_BYTES {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' exceeds the 1 MiB limit"
        )));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    let read_result = entry
        .by_ref()
        .take(MAX_ICON_BYTES + 1)
        .read_to_end(&mut bytes);
    if read_result.is_err() {
        return Err(ImportError::new(format!(
            "cannot decode archive icon '{entry_path}'"
        )));
    }
    if bytes.len() as u64 > MAX_ICON_BYTES {
        return Err(ImportError::new(format!(
            "archive icon '{entry_path}' exceeds the 1 MiB limit"
        )));
    }
    Ok(bytes)
}

fn sanitize_svg(
    original: &[u8],
    provider_id: &str,
    icon_slug: &str,
) -> Result<ProcessedIcon, ImportError> {
    let source = match std::str::from_utf8(original) {
        Ok(source) => source,
        Err(_) => return Err(ImportError::new("provider icon is not valid UTF-8 SVG")),
    };
    let source = strip_xml_declaration(source)?;
    let uppercase = source.to_ascii_uppercase();
    if uppercase.contains("<!DOCTYPE") || uppercase.contains("<!ENTITY") || source.contains("<?") {
        return Err(ImportError::new(
            "provider icon contains a forbidden XML declaration or entity",
        ));
    }
    let document = match Document::parse(source) {
        Ok(document) => document,
        Err(_) => return Err(ImportError::new("provider icon is not well-formed SVG")),
    };
    let root = document.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(ImportError::new(
            "provider icon root must use the canonical SVG namespace",
        ));
    }
    let view_box_value = match root.attribute("viewBox") {
        Some(value) => value,
        None => return Err(ImportError::new("provider icon is missing viewBox")),
    };
    let view_box = parse_view_box(view_box_value)?;
    let styles = collect_styles(&document)?;
    let (identifier_map, unused_identifiers) = identifier_map(&document, provider_id, icon_slug)?;
    let mut removed_metadata = false;
    for node in document.descendants() {
        if matches!(node.node_type(), NodeType::Comment | NodeType::PI)
            || (node.is_element() && matches!(node.tag_name().name(), "title" | "desc"))
        {
            removed_metadata = true;
            break;
        }
    }
    let mut state = SanitizeState {
        styles: &styles,
        identifiers: &identifier_map,
        removed_metadata,
        inlined_styles: !styles.is_empty(),
        removed_unused_identifiers: unused_identifiers,
    };
    let svg = match serialize_element(root, true, None, &mut state)? {
        Some(svg) => svg,
        None => {
            return Err(ImportError::new(
                "provider icon has no renderable SVG content",
            ));
        }
    };
    let mut transformations = Vec::new();
    if state.removed_metadata {
        transformations.push(ProviderPackTransformation::RemoveMetadata);
    }
    if state.inlined_styles {
        transformations.push(ProviderPackTransformation::InlineStyles);
    }
    if state.removed_unused_identifiers {
        transformations.push(ProviderPackTransformation::RemoveUnusedIdentifiers);
    }
    if !identifier_map.is_empty() {
        transformations.push(ProviderPackTransformation::NamespaceIdentifiers);
    }
    transformations.push(ProviderPackTransformation::NormalizeXml);

    Ok(ProcessedIcon {
        svg: format!("{svg}\n"),
        view_box,
        transformations,
    })
}

fn strip_xml_declaration(source: &str) -> Result<&str, ImportError> {
    let trimmed = source.trim_start_matches('\u{feff}').trim_start();
    let Some(after_prefix) = trimmed.strip_prefix("<?xml") else {
        return Ok(trimmed);
    };
    if !after_prefix.chars().next().is_some_and(char::is_whitespace) {
        return Ok(trimmed);
    }
    let end = match trimmed.find("?>") {
        Some(end) => end,
        None => {
            return Err(ImportError::new(
                "provider icon has an incomplete XML declaration",
            ));
        }
    };
    Ok(trimmed[end + 2..].trim_start())
}

fn parse_view_box(value: &str) -> Result<[i32; 4], ImportError> {
    let mut values = Vec::with_capacity(4);
    for component in
        value.split(|character: char| character.is_ascii_whitespace() || character == ',')
    {
        if component.is_empty() {
            continue;
        }
        match component.parse::<i32>() {
            Ok(component) => values.push(component),
            Err(_) => {
                return Err(ImportError::new(
                    "provider icon viewBox must contain four integers",
                ));
            }
        }
    }
    if values.len() != 4 || values[2] <= 0 || values[3] <= 0 {
        return Err(ImportError::new(
            "provider icon viewBox must contain four integers with positive dimensions",
        ));
    }
    Ok([values[0], values[1], values[2], values[3]])
}

fn collect_styles(document: &Document<'_>) -> Result<BTreeMap<String, String>, ImportError> {
    let mut styles = BTreeMap::new();
    for node in document.descendants() {
        if !node.is_element() || node.tag_name().name() != "style" {
            continue;
        }
        let stylesheet = match node.text() {
            Some(stylesheet) => stylesheet,
            None => return Err(ImportError::new("provider stylesheet is empty")),
        };
        for block in stylesheet.split('}') {
            let block = block.trim();
            if block.is_empty() {
                continue;
            }
            let (selector, declarations) = match block.split_once('{') {
                Some(parts) => parts,
                None => {
                    return Err(ImportError::new("provider stylesheet has invalid syntax"));
                }
            };
            let class_name = match selector.trim().strip_prefix('.') {
                Some(class_name) => class_name,
                None => {
                    return Err(ImportError::new(
                        "provider stylesheet selector is not a class",
                    ));
                }
            };
            if class_name.is_empty()
                || !class_name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(ImportError::new(
                    "provider stylesheet has an invalid class name",
                ));
            }
            let mut fill = None;
            for declaration in declarations.split(';') {
                let declaration = declaration.trim();
                if declaration.is_empty() {
                    continue;
                }
                let (property, value) = match declaration.split_once(':') {
                    Some(parts) => parts,
                    None => {
                        return Err(ImportError::new(
                            "provider stylesheet declaration is invalid",
                        ));
                    }
                };
                if property.trim() != "fill" || fill.is_some() {
                    return Err(ImportError::new(
                        "provider stylesheet may define one fill property per class",
                    ));
                }
                validate_safe_value(value.trim())?;
                fill = Some(value.trim().to_owned());
            }
            let fill = match fill {
                Some(fill) => fill,
                None => {
                    return Err(ImportError::new(
                        "provider stylesheet class does not define fill",
                    ));
                }
            };
            if styles.insert(class_name.to_owned(), fill).is_some() {
                return Err(ImportError::new(
                    "provider stylesheet declares a duplicate class",
                ));
            }
        }
    }
    Ok(styles)
}

fn identifier_map(
    document: &Document<'_>,
    provider_id: &str,
    icon_slug: &str,
) -> Result<(BTreeMap<String, String>, bool), ImportError> {
    let mut declared = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    for node in document.descendants() {
        if !node.is_element() {
            continue;
        }
        if let Some(identifier) = node.attribute("id") {
            if !declared.insert(identifier.to_owned()) {
                return Err(ImportError::new(
                    "provider icon declares a duplicate identifier",
                ));
            }
        }
        for attribute in node.attributes() {
            if let Some(identifier) = local_reference(attribute.value()) {
                referenced.insert(identifier.to_owned());
            } else if attribute.value().to_ascii_lowercase().contains("url(") {
                return Err(ImportError::new(
                    "provider icon contains a non-local URL reference",
                ));
            }
        }
    }
    for identifier in &referenced {
        if !declared.contains(identifier) {
            return Err(ImportError::new(
                "provider icon references an undeclared identifier",
            ));
        }
    }
    let mut map = BTreeMap::new();
    for (index, identifier) in referenced.iter().enumerate() {
        map.insert(
            identifier.clone(),
            format!("stack-{provider_id}-{icon_slug}-gradient-{index}"),
        );
    }
    let mut has_unused_identifier = false;
    for identifier in &declared {
        if !referenced.contains(identifier) {
            has_unused_identifier = true;
            break;
        }
    }
    Ok((map, has_unused_identifier))
}

fn local_reference(value: &str) -> Option<&str> {
    let value = value.strip_prefix("url(#")?;
    let value = value.strip_suffix(')')?;
    if value.is_empty() { None } else { Some(value) }
}

struct SanitizeState<'a> {
    styles: &'a BTreeMap<String, String>,
    identifiers: &'a BTreeMap<String, String>,
    removed_metadata: bool,
    inlined_styles: bool,
    removed_unused_identifiers: bool,
}

fn serialize_element(
    node: Node<'_, '_>,
    is_root: bool,
    parent_name: Option<&str>,
    state: &mut SanitizeState<'_>,
) -> Result<Option<String>, ImportError> {
    let name = node.tag_name().name();
    if matches!(name, "title" | "desc" | "style") {
        return Ok(None);
    }
    if !ALLOWED_ELEMENTS.contains(&name) {
        return Err(ImportError::new(format!(
            "provider icon element '{name}' is not allowed"
        )));
    }
    if node.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(ImportError::new(
            "provider icon contains a foreign XML namespace",
        ));
    }
    if name == "svg" && !is_root {
        return Err(ImportError::new("nested SVG elements are not allowed"));
    }
    if name == "defs" && parent_name != Some("svg") {
        return Err(ImportError::new("defs must be a direct child of SVG"));
    }
    if matches!(name, "linearGradient" | "radialGradient") && parent_name != Some("defs") {
        return Err(ImportError::new(
            "gradients must be direct children of defs",
        ));
    }
    if name == "stop" && !matches!(parent_name, Some("linearGradient" | "radialGradient")) {
        return Err(ImportError::new(
            "gradient stops must be direct children of gradients",
        ));
    }
    if parent_name == Some("defs") && !matches!(name, "linearGradient" | "radialGradient") {
        return Err(ImportError::new("defs may contain only gradients"));
    }

    let mut attributes = BTreeMap::new();
    for attribute in node.attributes() {
        if attribute.namespace().is_some() {
            return Err(ImportError::new(
                "provider icon contains a namespaced attribute",
            ));
        }
        let attribute_name = attribute.name();
        if attribute_name.starts_with("on") {
            return Err(ImportError::new(
                "provider icon contains an event handler attribute",
            ));
        }
        match attribute_name {
            "version" => continue,
            "class" => {
                let fill = match state.styles.get(attribute.value()) {
                    Some(fill) => fill,
                    None => {
                        return Err(ImportError::new(
                            "provider icon references an unknown stylesheet class",
                        ));
                    }
                };
                attributes.insert("fill".to_owned(), fill.clone());
                continue;
            }
            "id" => {
                if let Some(identifier) = state.identifiers.get(attribute.value()) {
                    if !matches!(name, "linearGradient" | "radialGradient") {
                        return Err(ImportError::new(
                            "only gradients may retain provider icon identifiers",
                        ));
                    }
                    attributes.insert("id".to_owned(), identifier.clone());
                } else {
                    state.removed_unused_identifiers = true;
                }
                continue;
            }
            _ => {}
        }
        if !ALLOWED_ATTRIBUTES.contains(&attribute_name) {
            return Err(ImportError::new(format!(
                "provider icon attribute '{attribute_name}' is not allowed"
            )));
        }
        let value = if let Some(identifier) = local_reference(attribute.value()) {
            let mapped = match state.identifiers.get(identifier) {
                Some(mapped) => mapped,
                None => {
                    return Err(ImportError::new(
                        "provider icon references an undeclared identifier",
                    ));
                }
            };
            if !matches!(attribute_name, "fill" | "stroke") {
                return Err(ImportError::new(
                    "local references are allowed only for fill or stroke",
                ));
            }
            format!("url(#{mapped})")
        } else {
            validate_safe_value(attribute.value())?;
            attribute.value().to_owned()
        };
        if attributes
            .insert(attribute_name.to_owned(), value)
            .is_some()
        {
            return Err(ImportError::new(
                "provider icon declares a duplicate effective attribute",
            ));
        }
    }

    let mut children = String::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Element => {
                if let Some(serialized) = serialize_element(child, false, Some(name), state)? {
                    children.push_str(&serialized);
                }
            }
            NodeType::Text if child.text().unwrap_or_default().trim().is_empty() => {}
            NodeType::Comment | NodeType::PI => state.removed_metadata = true,
            _ => {
                return Err(ImportError::new(
                    "provider icon contains unsupported visible text or XML content",
                ));
            }
        }
    }
    if name == "defs" && children.is_empty() {
        return Ok(None);
    }

    let mut output = String::new();
    output.push('<');
    output.push_str(name);
    if is_root {
        output.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
    }
    for (name, value) in attributes {
        output.push(' ');
        output.push_str(&name);
        output.push_str("=\"");
        output.push_str(&escape_xml_attribute(&value));
        output.push('"');
    }
    if children.is_empty() {
        output.push_str("/>");
    } else {
        output.push('>');
        output.push_str(&children);
        output.push_str("</");
        output.push_str(name);
        output.push('>');
    }
    Ok(Some(output))
}

fn validate_safe_value(value: &str) -> Result<(), ImportError> {
    let lowercase = value.to_ascii_lowercase();
    if lowercase.contains("url(")
        || lowercase.contains("javascript:")
        || lowercase.contains("data:")
        || lowercase.contains("http://")
        || lowercase.contains("https://")
        || lowercase.contains("//")
    {
        return Err(ImportError::new(
            "provider icon contains an external or executable reference",
        ));
    }
    Ok(())
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn write_pack_directory(
    root: &Path,
    manifest: &[u8],
    notice: &[u8],
    assets: &[(String, String)],
) -> Result<(), ImportError> {
    if fs::create_dir(root.join("assets")).is_err() {
        return Err(ImportError::new(
            "cannot create provider pack asset directory",
        ));
    }
    write_new_file(&root.join("manifest.json"), manifest)?;
    write_new_file(&root.join("NOTICE.md"), notice)?;
    for (relative_path, svg) in assets {
        write_new_file(&root.join(relative_path), svg.as_bytes())?;
    }
    Ok(())
}

fn write_new_file(path: &Path, contents: &[u8]) -> Result<(), ImportError> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(_) => return Err(ImportError::new("cannot create provider pack file")),
    };
    if file.write_all(contents).is_err() || file.sync_all().is_err() {
        return Err(ImportError::new("cannot write provider pack file"));
    }
    Ok(())
}

fn create_temporary_directory(parent: &Path) -> Result<PathBuf, ImportError> {
    for attempt in 0..128_u8 {
        let path = parent.join(format!(
            ".stack-provider-pack-{}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(ImportError::new(format!(
                    "cannot create temporary provider pack directory: {}",
                    stable_io_error(error.kind())
                )));
            }
        }
    }
    Err(ImportError::new(
        "cannot reserve a temporary provider pack directory",
    ))
}

fn render_notice(manifest: &ProviderPack) -> String {
    let mut notice = format!(
        "# Stack provider icon pack notice\n\nProvider: {} (`{}`)\n\nSource: <{}>\n\nOfficial archive: <{}>\n\nRelease: {}\n\nArchive SHA-256: `{}`\n\nTerms: <{}>\n\nTerms reviewed: {} (review again after {})\n\n{}\n\n{}\n\n{}\n\nThis local pack was created from an archive selected by the user. Stack does not redistribute these asset bytes. Use and distribute generated diagrams only as permitted by the linked provider terms.\n\n## Icons\n",
        manifest.provider.name,
        manifest.provider.id,
        manifest.source.page_url,
        manifest.source.archive_url,
        manifest.source.release,
        manifest.source.archive_sha256,
        manifest.source.terms_url,
        manifest.source.terms_reviewed_at,
        manifest.source.review_after,
        manifest.notice.attribution,
        manifest.notice.terms_summary,
        manifest.notice.non_endorsement,
    );
    for icon in &manifest.icons {
        notice.push_str(&format!("\n- `{}`: {}\n", icon.id, icon.product_name));
    }
    notice
}

fn sha256(bytes: &[u8]) -> String {
    let mut output = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        let _ = fmt::Write::write_fmt(&mut output, format_args!("{byte:02x}"));
    }
    output
}

fn stable_io_error(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "file not found",
        std::io::ErrorKind::PermissionDenied => "permission denied",
        std::io::ErrorKind::InvalidData => "invalid data",
        _ => "I/O error",
    }
}

fn provider_profile(provider: &str) -> Option<ProviderProfile<'static>> {
    match provider {
        "aws" => Some(AWS_PROFILE),
        "gcp" => Some(GCP_PROFILE),
        "azure" => Some(AZURE_PROFILE),
        _ => None,
    }
}

const AWS_OUTPUTS: &[ProviderPackPermittedOutput] = &[
    ProviderPackPermittedOutput::ArchitectureDiagram,
    ProviderPackPermittedOutput::Whitepaper,
    ProviderPackPermittedOutput::Presentation,
    ProviderPackPermittedOutput::DataSheet,
    ProviderPackPermittedOutput::Poster,
];

const AWS_ICONS: &[IconProfile<'static>] = &[
    IconProfile {
        slug: "s3",
        subject: "Object storage service",
        product_name: "Amazon Simple Storage Service (Amazon S3)",
        node_kind: ProviderNodeKind::Storage,
        archive_path: "Architecture-Service-Icons_07312026/Arch_Storage/48/Arch_Amazon-Simple-Storage-Service_48.svg",
    },
    IconProfile {
        slug: "sqs",
        subject: "Managed message queue",
        product_name: "Amazon Simple Queue Service (Amazon SQS)",
        node_kind: ProviderNodeKind::Queue,
        archive_path: "Architecture-Service-Icons_07312026/Arch_Application-Integration/48/Arch_Amazon-Simple-Queue-Service_48.svg",
    },
    IconProfile {
        slug: "lambda",
        subject: "Serverless function service",
        product_name: "AWS Lambda",
        node_kind: ProviderNodeKind::Function,
        archive_path: "Architecture-Service-Icons_07312026/Arch_Compute/48/Arch_AWS-Lambda_48.svg",
    },
    IconProfile {
        slug: "ec2",
        subject: "Virtual compute service",
        product_name: "Amazon Elastic Compute Cloud (Amazon EC2)",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Architecture-Service-Icons_07312026/Arch_Compute/48/Arch_Amazon-EC2_48.svg",
    },
    IconProfile {
        slug: "rds",
        subject: "Managed relational database service",
        product_name: "Amazon Relational Database Service (Amazon RDS)",
        node_kind: ProviderNodeKind::Database,
        archive_path: "Architecture-Service-Icons_07312026/Arch_Databases/48/Arch_Amazon-RDS_48.svg",
    },
    IconProfile {
        slug: "dynamodb",
        subject: "Managed NoSQL database service",
        product_name: "Amazon DynamoDB",
        node_kind: ProviderNodeKind::Database,
        archive_path: "Architecture-Service-Icons_07312026/Arch_Databases/48/Arch_Amazon-DynamoDB_48.svg",
    },
    IconProfile {
        slug: "eks",
        subject: "Managed Kubernetes service",
        product_name: "Amazon Elastic Kubernetes Service (Amazon EKS)",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Architecture-Service-Icons_07312026/Arch_Containers/48/Arch_Amazon-Elastic-Kubernetes-Service_48.svg",
    },
];

const AWS_PROFILE: ProviderProfile<'static> = ProviderProfile {
    id: "aws",
    name: "Amazon Web Services",
    page_url: "https://aws.amazon.com/architecture/icons/",
    archive_url: "https://d1.awsstatic.com/onedam/marketing-channels/website/public/shared/architecture-icon-release/Icon-package_07312026.5846e92413caa21490223536cc97f1269e44fa92.zip",
    archive_sha256: "sha256:d2d166c453526471749d520e0db022c459abef759d2946cf2dd1d1c992dc6526",
    release: "Icon-package_07312026",
    retrieved_at: "2026-09-04",
    terms_url: "https://aws.amazon.com/trademark-guidelines/",
    terms_reviewed_at: "2026-09-04",
    review_after: "2026-12-03",
    copyright: "Copyright Amazon Web Services, Inc. or its affiliates",
    license_id: "LicenseRef-AWS-Architecture-Icons-Terms",
    archive_license_included: false,
    permitted_outputs: AWS_OUTPUTS,
    product_name_nearby: true,
    attribution: "AWS architecture icons are owned by Amazon Web Services, Inc. or its affiliates.",
    terms_summary: "Use is limited to the architecture-diagram materials described by the official AWS Architecture Icons page and applicable AWS trademark guidelines.",
    non_endorsement: "Amazon Web Services does not sponsor or endorse this diagram or Stack.",
    icons: AWS_ICONS,
};

const GCP_OUTPUTS: &[ProviderPackPermittedOutput] = &[
    ProviderPackPermittedOutput::ArchitectureDiagram,
    ProviderPackPermittedOutput::Documentation,
];

const GCP_ICONS: &[IconProfile<'static>] = &[
    IconProfile {
        slug: "cloud-run",
        subject: "Managed application runtime",
        product_name: "Cloud Run",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Unique Icons/Cloud Run/SVG/CloudRun-512-color-rgb.svg",
    },
    IconProfile {
        slug: "cloud-storage",
        subject: "Object storage service",
        product_name: "Cloud Storage",
        node_kind: ProviderNodeKind::Storage,
        archive_path: "Unique Icons/Cloud Storage/SVG/Cloud_Storage-512-color.svg",
    },
    IconProfile {
        slug: "compute-engine",
        subject: "Virtual compute service",
        product_name: "Compute Engine",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Unique Icons/Compute Engine/SVG/ComputeEngine-512-color-rgb.svg",
    },
    IconProfile {
        slug: "gke",
        subject: "Managed Kubernetes service",
        product_name: "Google Kubernetes Engine",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Unique Icons/GKE/SVG/GKE-512-color.svg",
    },
    IconProfile {
        slug: "bigquery",
        subject: "Managed analytics data warehouse",
        product_name: "BigQuery",
        node_kind: ProviderNodeKind::Database,
        archive_path: "Unique Icons/BigQuery/SVG/BigQuery-512-color.svg",
    },
    IconProfile {
        slug: "cloud-sql",
        subject: "Managed relational database service",
        product_name: "Cloud SQL",
        node_kind: ProviderNodeKind::Database,
        archive_path: "Unique Icons/Cloud SQL/SVG/CloudSQL-512-color.svg",
    },
];

const GCP_PROFILE: ProviderProfile<'static> = ProviderProfile {
    id: "gcp",
    name: "Google Cloud",
    page_url: "https://cloud.google.com/icons",
    archive_url: "https://services.google.com/fh/files/misc/core-products-icons.zip",
    archive_sha256: "sha256:6531a10f58bc599c24d9a455d81dd757c1a03c3c43da9cddf639b859c1c1eece",
    release: "Core product icons (May 2026 guide)",
    retrieved_at: "2026-09-04",
    terms_url: "https://about.google/brand-resource-center/",
    terms_reviewed_at: "2026-09-04",
    review_after: "2026-12-03",
    copyright: "Copyright Google LLC",
    license_id: "LicenseRef-Google-Cloud-Product-Icons-Terms",
    archive_license_included: false,
    permitted_outputs: GCP_OUTPUTS,
    product_name_nearby: true,
    attribution: "Google Cloud product icons are owned by Google LLC.",
    terms_summary: "Use is limited to diagrams and technical documentation described by the official Google Cloud Icon Library and applicable Google brand terms.",
    non_endorsement: "Google does not sponsor or endorse this diagram or Stack.",
    icons: GCP_ICONS,
};

const AZURE_OUTPUTS: &[ProviderPackPermittedOutput] = &[
    ProviderPackPermittedOutput::ArchitectureDiagram,
    ProviderPackPermittedOutput::TrainingMaterial,
    ProviderPackPermittedOutput::Documentation,
];

const AZURE_ICONS: &[IconProfile<'static>] = &[
    IconProfile {
        slug: "virtual-machines",
        subject: "Virtual compute service",
        product_name: "Azure Virtual Machines",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Azure_Public_Service_Icons/Icons/compute/10021-icon-service-Virtual-Machine.svg",
    },
    IconProfile {
        slug: "storage-accounts",
        subject: "Cloud storage account",
        product_name: "Azure Storage Accounts",
        node_kind: ProviderNodeKind::Storage,
        archive_path: "Azure_Public_Service_Icons/Icons/storage/10086-icon-service-Storage-Accounts.svg",
    },
    IconProfile {
        slug: "azure-sql-database",
        subject: "Managed relational database service",
        product_name: "Azure SQL Database",
        node_kind: ProviderNodeKind::Database,
        archive_path: "Azure_Public_Service_Icons/Icons/databases/10130-icon-service-SQL-Database.svg",
    },
    IconProfile {
        slug: "aks",
        subject: "Managed Kubernetes service",
        product_name: "Azure Kubernetes Service (AKS)",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Azure_Public_Service_Icons/Icons/containers/10023-icon-service-Kubernetes-Services.svg",
    },
    IconProfile {
        slug: "app-service",
        subject: "Managed application platform",
        product_name: "Azure App Service",
        node_kind: ProviderNodeKind::Service,
        archive_path: "Azure_Public_Service_Icons/Icons/app services/10035-icon-service-App-Services.svg",
    },
];

const AZURE_PROFILE: ProviderProfile<'static> = ProviderProfile {
    id: "azure",
    name: "Microsoft Azure",
    page_url: "https://learn.microsoft.com/azure/architecture/icons/",
    archive_url: "https://arch-center.azureedge.net/icons/Azure_Public_Service_Icons_V24.zip",
    archive_sha256: "sha256:921594ccd1bf3d9c0a1bd7b6d924e050551a59342f2b353bb74bdcf761c35141",
    release: "Azure_Public_Service_Icons_V24",
    retrieved_at: "2026-09-04",
    terms_url: "https://learn.microsoft.com/azure/architecture/icons/",
    terms_reviewed_at: "2026-09-04",
    review_after: "2026-12-03",
    copyright: "Copyright Microsoft Corporation",
    license_id: "LicenseRef-Microsoft-Azure-Architecture-Icons-Terms",
    archive_license_included: true,
    permitted_outputs: AZURE_OUTPUTS,
    product_name_nearby: true,
    attribution: "Azure architecture icons are owned by Microsoft Corporation.",
    terms_summary: "Use is limited to architecture diagrams, training materials, and documentation under the terms included in the official Azure icon archive.",
    non_endorsement: "Microsoft does not sponsor or endorse this diagram or Stack.",
    icons: AZURE_ICONS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    const TEST_OUTPUTS: &[ProviderPackPermittedOutput] =
        &[ProviderPackPermittedOutput::ArchitectureDiagram];

    fn temporary_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("stack-cli-provider-{label}-{}", std::process::id()))
    }

    fn zip_with_entry(name: &str, contents: &[u8]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        assert!(
            writer
                .start_file(name, SimpleFileOptions::default())
                .is_ok()
        );
        assert!(writer.write_all(contents).is_ok());
        let result = writer.finish();
        assert!(result.is_ok());
        result.map(Cursor::into_inner).unwrap_or_default()
    }

    fn zip_with_directory(name: &str) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        assert!(
            writer
                .add_directory(name, SimpleFileOptions::default())
                .is_ok()
        );
        let result = writer.finish();
        assert!(result.is_ok());
        result.map(Cursor::into_inner).unwrap_or_default()
    }

    fn assert_svg_error(source: &[u8], expected: &str) {
        let error = sanitize_svg(source, "example", "fixture").err();
        assert!(
            error.is_some_and(|error| error.to_string().contains(expected)),
            "expected SVG error containing: {expected}"
        );
    }

    fn read_test_icon(archive: Vec<u8>, entry_path: &str) -> Result<Vec<u8>, ImportError> {
        let mut archive = match ZipArchive::new(Cursor::new(archive)) {
            Ok(archive) => archive,
            Err(_) => return Err(ImportError::new("invalid test archive")),
        };
        read_icon_entry(&mut archive, entry_path)
    }

    fn test_profile<'a>(archive_sha256: &'a str, archive_path: &'a str) -> ProviderProfile<'a> {
        let icons = Box::leak(Box::new([IconProfile {
            slug: "storage",
            subject: "Object storage service",
            product_name: "Example Storage",
            node_kind: ProviderNodeKind::Storage,
            archive_path,
        }]));
        ProviderProfile {
            id: "example",
            name: "Example Cloud",
            page_url: "https://example.com/icons",
            archive_url: "https://example.com/icons.zip",
            archive_sha256,
            release: "fixture-1",
            retrieved_at: "2026-09-04",
            terms_url: "https://example.com/terms",
            terms_reviewed_at: "2026-09-04",
            review_after: "2026-12-03",
            copyright: "Copyright Example Cloud",
            license_id: "LicenseRef-Example-Icons",
            archive_license_included: false,
            permitted_outputs: TEST_OUTPUTS,
            product_name_nearby: true,
            attribution: "Example Cloud owns the icons.",
            terms_summary: "Architecture diagram use only.",
            non_endorsement: "Example Cloud does not endorse Stack.",
            icons,
        }
    }

    #[test]
    fn audited_provider_profiles_have_expected_coverage() {
        assert_eq!(
            provider_profile("aws").map(|profile| profile.icons.len()),
            Some(7)
        );
        assert_eq!(
            provider_profile("gcp").map(|profile| profile.icons.len()),
            Some(6)
        );
        assert_eq!(
            provider_profile("azure").map(|profile| profile.icons.len()),
            Some(5)
        );
        assert!(provider_profile("unknown").is_none());
    }

    #[test]
    fn sanitizer_preserves_colors_and_namespaces_gradients() {
        let source = br##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" version="1.1" viewBox="0 0 18 18"><defs><linearGradient id="paint" x1="0" y1="0" x2="18" y2="18"><stop offset="0" stop-color="#37c2b1"/><stop offset="1" stop-color="#258277"/></linearGradient></defs><title>Storage</title><path id="unused" fill="url(#paint)" d="M0 0h18v18H0z"/></svg>"##;
        let processed = sanitize_svg(source, "azure", "storage-accounts");
        assert!(processed.is_ok());
        let Ok(processed) = processed else {
            return;
        };
        assert_eq!(processed.view_box, [0, 0, 18, 18]);
        assert!(processed.svg.contains("#37c2b1"));
        assert!(processed.svg.contains("#258277"));
        assert!(
            processed
                .svg
                .contains("id=\"stack-azure-storage-accounts-gradient-0\"")
        );
        assert!(
            processed
                .svg
                .contains("fill=\"url(#stack-azure-storage-accounts-gradient-0)\"")
        );
        assert!(!processed.svg.contains("<title"));
        assert!(!processed.svg.contains("id=\"unused\""));
        assert!(
            processed
                .transformations
                .contains(&ProviderPackTransformation::NamespaceIdentifiers)
        );
    }

    #[test]
    fn sanitizer_inlines_the_audited_google_fill_classes() {
        let source = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><defs><style>.st0 { fill: none; } .st1 { fill: #4285f4; }</style></defs><rect class="st0" width="512" height="512"/><path class="st1" d="M1 1h10v10z"/></svg>"##;
        let processed = sanitize_svg(source, "gcp", "cloud-run");
        assert!(processed.is_ok());
        let Ok(processed) = processed else {
            return;
        };
        assert!(processed.svg.contains("fill=\"none\""));
        assert!(processed.svg.contains("fill=\"#4285f4\""));
        assert!(!processed.svg.contains("style"));
        assert!(!processed.svg.contains("class"));
        assert!(
            processed
                .transformations
                .contains(&ProviderPackTransformation::InlineStyles)
        );
    }

    #[test]
    fn sanitizer_rejects_active_or_external_svg_content() {
        for source in [
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><script>alert(1)</script></svg>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path onload="alert(1)" d="M0 0"/></svg>"#.as_slice(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="url(https://example.com/a.svg#x)" d="M0 0"/></svg>"#.as_slice(),
            br#"<!DOCTYPE svg><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"/>"#.as_slice(),
        ] {
            assert!(sanitize_svg(source, "example", "unsafe").is_err());
        }
    }

    #[test]
    fn sanitizer_rejects_invalid_xml_roots_and_view_boxes() {
        for (source, expected) in [
            (b"\xff".as_slice(), "not valid UTF-8"),
            (
                b"<?xml version=\"1.0\"".as_slice(),
                "incomplete XML declaration",
            ),
            (b"<svg".as_slice(), "not well-formed SVG"),
            (
                br#"<g xmlns="http://www.w3.org/2000/svg"/>"#.as_slice(),
                "canonical SVG namespace",
            ),
            (
                br#"<svg viewBox="0 0 24 24"/>"#.as_slice(),
                "canonical SVG namespace",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.as_slice(),
                "missing viewBox",
            ),
            (
                br#"<?xml-stylesheet href="https://example.com/style.css"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"/>"#.as_slice(),
                "forbidden XML declaration",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 wide 24"/>"#.as_slice(),
                "four integers",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0,,0,24,0"/>"#.as_slice(),
                "positive dimensions",
            ),
            (
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24"/>"#.as_slice(),
                "positive dimensions",
            ),
        ] {
            assert_svg_error(source, expected);
        }
    }

    #[test]
    fn sanitizer_rejects_unsupported_stylesheets() {
        for (style, expected) in [
            ("", "stylesheet is empty"),
            ("bad", "invalid syntax"),
            ("path{fill:red}", "selector is not a class"),
            (".!{fill:red}", "invalid class name"),
            (".a{fill}", "declaration is invalid"),
            (".a{stroke:red}", "one fill property"),
            (".a{fill:red;fill:blue}", "one fill property"),
            (".a{}", "does not define fill"),
            (".a{fill:red}.a{fill:blue}", "duplicate class"),
            (".a{fill:javascript:alert(1)}", "external or executable"),
        ] {
            let source = format!(
                "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"><defs><style>{style}</style></defs><path class=\"a\" d=\"M0 0\"/></svg>"
            );
            assert_svg_error(source.as_bytes(), expected);
        }
        assert_svg_error(
            format!(
                "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"><defs><style/></defs></svg>"
            )
            .as_bytes(),
            "stylesheet is empty",
        );
    }

    #[test]
    fn sanitizer_rejects_unsafe_structure_attributes_and_identifiers() {
        for (body, expected) in [
            (
                "<defs><linearGradient id=\"a\"/><linearGradient id=\"a\"/></defs>",
                "duplicate identifier",
            ),
            (
                "<path fill=\"url(#missing)\" d=\"M0 0\"/>",
                "undeclared identifier",
            ),
            (
                "<foreign:path xmlns:foreign=\"https://example.com/ns\"/>",
                "foreign XML namespace",
            ),
            ("<svg viewBox=\"0 0 1 1\"/>", "nested SVG"),
            ("<g><defs/></g>", "defs must be a direct child"),
            ("<linearGradient/>", "gradients must be direct children"),
            ("<g><stop/></g>", "gradient stops must be direct children"),
            (
                "<defs><path d=\"M0 0\"/></defs>",
                "defs may contain only gradients",
            ),
            ("<path xml:lang=\"en\" d=\"M0 0\"/>", "namespaced attribute"),
            (
                "<path class=\"missing\" d=\"M0 0\"/>",
                "unknown stylesheet class",
            ),
            (
                "<path id=\"shape\" d=\"M0 0\"/><path fill=\"url(#shape)\" d=\"M0 0\"/>",
                "only gradients may retain",
            ),
            (
                "<path unknown=\"value\" d=\"M0 0\"/>",
                "attribute 'unknown' is not allowed",
            ),
            (
                "<defs><linearGradient id=\"paint\"/></defs><path d=\"url(#paint)\"/>",
                "only for fill or stroke",
            ),
            ("<path>visible</path>", "unsupported visible text"),
            (
                "<path fill=\"https://example.com/icon.svg\" d=\"M0 0\"/>",
                "external or executable",
            ),
        ] {
            let source =
                format!("<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\">{body}</svg>");
            assert_svg_error(source.as_bytes(), expected);
        }

        let duplicate_fill = format!(
            "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"><defs><style>.a{{fill:red}}</style></defs><path class=\"a\" fill=\"blue\" d=\"M0 0\"/></svg>"
        );
        assert_svg_error(
            &duplicate_fill.into_bytes(),
            "duplicate effective attribute",
        );
    }

    #[test]
    fn sanitizer_removes_empty_definitions_and_escapes_attributes() {
        let source = format!(
            "<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0, 0, 24, 24\"><defs><title>metadata</title></defs><path aria-hidden=\"a&amp;b&lt;c&gt;d&quot;e\" d=\"M0 0\"/></svg>"
        );
        let processed = sanitize_svg(source.as_bytes(), "example", "escape");
        assert!(processed.is_ok());
        let Some(processed) = processed.ok() else {
            return;
        };
        assert!(!processed.svg.contains("<defs"));
        assert!(processed.svg.contains("a&amp;b&lt;c&gt;d&quot;e"));
        assert!(
            processed
                .transformations
                .contains(&ProviderPackTransformation::RemoveMetadata)
        );
    }

    #[test]
    fn archive_entry_checks_reject_missing_directories_and_large_icons() {
        let archive = zip_with_entry("actual.svg", b"svg");
        assert!(
            read_test_icon(archive, "missing.svg")
                .err()
                .is_some_and(|error| error.to_string().contains("missing required icon"))
        );

        let archive = zip_with_directory("icons/");
        assert!(
            read_test_icon(archive, "icons/")
                .err()
                .is_some_and(|error| error.to_string().contains("not a regular file"))
        );

        let archive = zip_with_entry("large.svg", &vec![b'x'; MAX_ICON_BYTES as usize + 1]);
        assert!(
            read_test_icon(archive, "large.svg")
                .err()
                .is_some_and(|error| error.to_string().contains("1 MiB limit"))
        );
    }

    #[test]
    fn local_import_rejects_invalid_archive_inputs_and_output_parent() {
        let root = temporary_root("invalid-inputs");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&root);
        assert!(fs::create_dir(&root).is_ok());
        let output_path = root.join("pack");

        assert!(
            import_provider_pack("unknown", &root.join("missing.zip"), &output_path)
                .err()
                .is_some_and(|error| error.to_string().contains("unknown provider"))
        );
        assert!(
            import_provider_pack("aws", &root.join("missing.zip"), &output_path)
                .err()
                .is_some_and(|error| error.to_string().contains("file not found"))
        );
        assert!(
            import_profile(test_profile("sha256:none", "icon.svg"), &root, &output_path)
                .err()
                .is_some_and(|error| error.to_string().contains("not a file"))
        );

        let large_archive_path = root.join("large.zip");
        let large_archive = fs::File::create(&large_archive_path);
        assert!(large_archive.is_ok());
        if let Ok(large_archive) = large_archive {
            assert!(large_archive.set_len(MAX_ARCHIVE_BYTES + 1).is_ok());
        }
        assert!(
            import_profile(
                test_profile("sha256:none", "icon.svg"),
                &large_archive_path,
                &output_path,
            )
            .err()
            .is_some_and(|error| error.to_string().contains("32 MiB limit"))
        );

        let invalid_archive_path = root.join("invalid.zip");
        let invalid_archive = b"not a zip archive";
        assert!(fs::write(&invalid_archive_path, invalid_archive).is_ok());
        let invalid_digest = sha256(invalid_archive);
        assert!(
            import_profile(
                test_profile(&invalid_digest, "icon.svg"),
                &invalid_archive_path,
                &output_path,
            )
            .err()
            .is_some_and(|error| error.to_string().contains("not a supported ZIP"))
        );

        let valid_archive_path = root.join("valid.zip");
        let valid_archive = zip_with_entry(
            "icon.svg",
            format!("<svg xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 24 24\"/>").as_bytes(),
        );
        assert!(fs::write(&valid_archive_path, &valid_archive).is_ok());
        let parent_file = root.join("not-a-directory");
        assert!(fs::write(&parent_file, b"file").is_ok());
        assert!(
            import_profile(
                test_profile(&sha256(&valid_archive), "icon.svg"),
                &valid_archive_path,
                &parent_file.join("pack"),
            )
            .err()
            .is_some_and(|error| error.to_string().contains("not a directory"))
        );
        assert!(fs::remove_dir_all(&root).is_ok());
    }

    #[test]
    fn pack_writes_and_temporary_directories_fail_closed() {
        let root = temporary_root("write-failures");
        let _ = fs::remove_dir_all(&root);
        assert!(fs::create_dir(&root).is_ok());

        assert!(fs::create_dir(root.join("assets")).is_ok());
        assert!(
            write_pack_directory(&root, b"{}", b"notice", &[])
                .err()
                .is_some_and(|error| error.to_string().contains("asset directory"))
        );
        let existing = root.join("existing");
        assert!(fs::write(&existing, b"existing").is_ok());
        assert!(
            write_new_file(&existing, b"replacement")
                .err()
                .is_some_and(|error| error.to_string().contains("cannot create"))
        );
        assert!(
            create_temporary_directory(&root.join("missing"))
                .err()
                .is_some_and(|error| error.to_string().contains("cannot create temporary"))
        );

        let collision_root = root.join("collisions");
        assert!(fs::create_dir(&collision_root).is_ok());
        for attempt in 0..128_u8 {
            assert!(
                fs::create_dir(collision_root.join(format!(
                    ".stack-provider-pack-{}-{attempt}",
                    std::process::id()
                )))
                .is_ok()
            );
        }
        assert!(
            create_temporary_directory(&collision_root)
                .err()
                .is_some_and(|error| error.to_string().contains("cannot reserve"))
        );
        assert_eq!(
            stable_io_error(std::io::ErrorKind::PermissionDenied),
            "permission denied"
        );
        assert_eq!(
            stable_io_error(std::io::ErrorKind::InvalidData),
            "invalid data"
        );
        assert_eq!(stable_io_error(std::io::ErrorKind::Other), "I/O error");
        assert!(fs::remove_dir_all(&root).is_ok());
    }

    #[test]
    fn local_import_writes_manifest_notice_and_sanitized_asset_atomically() {
        let root = temporary_root("success");
        let archive_path = root.with_extension("zip");
        let output_path = root.join("pack");
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#123456" d="M0 0h24v24H0z"/></svg>"##;
        let archive = zip_with_entry("icons/storage.svg", svg);
        assert!(fs::write(&archive_path, &archive).is_ok());
        assert!(fs::create_dir(&root).is_ok());
        let digest = sha256(&archive);
        let imported = import_profile(
            test_profile(&digest, "icons/storage.svg"),
            &archive_path,
            &output_path,
        );
        assert!(imported.is_ok());
        let Ok(imported) = imported else {
            return;
        };
        assert_eq!(imported.icon_count, 1);
        let manifest = fs::read_to_string(&imported.manifest_path);
        assert!(manifest.is_ok());
        let Some(manifest): Option<ProviderPack> = manifest
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
        else {
            let _ = fs::remove_dir_all(&root);
            let _ = fs::remove_file(&archive_path);
            return;
        };
        assert_eq!(manifest.provider.id, "example");
        assert_eq!(manifest.icons[0].id, "example:storage");
        assert_eq!(manifest.icons[0].asset.original_sha256, sha256(svg));
        assert!(output_path.join("assets/storage.svg").is_file());
        assert!(
            fs::read_to_string(imported.notice_path)
                .unwrap_or_default()
                .contains("Stack does not redistribute these asset bytes")
        );
        assert!(fs::remove_dir_all(&root).is_ok());
        assert!(fs::remove_file(&archive_path).is_ok());
    }

    #[test]
    fn local_import_rejects_hash_mismatch_existing_output_and_unsafe_paths() {
        let root = temporary_root("failures");
        let archive_path = root.with_extension("zip");
        let output_path = root.join("pack");
        let archive = zip_with_entry("../unsafe.svg", b"not svg");
        assert!(fs::write(&archive_path, &archive).is_ok());
        assert!(fs::create_dir(&root).is_ok());

        let mismatch = import_profile(
            test_profile(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "../unsafe.svg",
            ),
            &archive_path,
            &output_path,
        );
        assert!(
            mismatch
                .err()
                .is_some_and(|error| error.to_string().contains("archive hash does not match"))
        );

        let digest = sha256(&archive);
        let unsafe_path = import_profile(
            test_profile(&digest, "../unsafe.svg"),
            &archive_path,
            &output_path,
        );
        assert!(
            unsafe_path
                .err()
                .is_some_and(|error| error.to_string().contains("unsafe path"))
        );

        assert!(fs::create_dir(&output_path).is_ok());
        let existing = import_profile(
            test_profile(&digest, "../unsafe.svg"),
            &archive_path,
            &output_path,
        );
        assert!(
            existing
                .err()
                .is_some_and(|error| error.to_string().contains("already exists"))
        );
        assert!(fs::remove_dir_all(&root).is_ok());
        assert!(fs::remove_file(&archive_path).is_ok());
    }
}
