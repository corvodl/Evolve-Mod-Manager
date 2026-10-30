//! Bounded, read-only inspection of CryEngine surface and material-effect data.
//!
//! The three authored documents are treated as untrusted input:
//!
//! * `Libs/MaterialEffects/SurfaceTypes.xml` (`<SurfaceTypes>`),
//! * `Libs/MaterialEffects/MaterialEffects.xml` (Excel 2003 workbook XML),
//! * `Libs/MaterialEffects/FXLibs/*.xml` (`<FXLib>`).
//!
//! Nothing is executed, no referenced resource is opened, and no input bytes are
//! rewritten. The parser deliberately keeps three states distinct:
//!
//! * a sparse/missing matrix cell (`MatrixLookup::Missing`),
//! * an authored empty cell (`MatrixCellValue::Blank`, which means "no effect"),
//! * an explicitly requested `mat_default` surface or breakage fallback.
//!
//! A missing cell is never silently replaced with `mat_default`, and surface
//! names are matched case-insensitively but never fuzzily
//! (`mat_metal_extra` is not `mat_metal`).

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

/// Default maximum size of one authored XML document.
pub const MAX_XML_BYTES: usize = 8 * 1024 * 1024;
/// Default maximum number of XML elements in one document.
pub const MAX_XML_NODES: usize = 200_000;
/// Default maximum XML nesting depth.
pub const MAX_XML_DEPTH: usize = 64;
/// Default maximum number of attributes in one document.
pub const MAX_XML_ATTRIBUTES: usize = 200_000;
/// Default maximum number of `SurfaceType` entries.
pub const MAX_SURFACE_TYPES: usize = 4_096;
/// Default maximum number of workbook data rows (the header is separate).
pub const MAX_MATRIX_ROWS: usize = 8_192;
/// Default maximum number of workbook columns.
pub const MAX_MATRIX_COLUMNS: usize = 1_024;
/// Default maximum number of effects in one FXLib (including nested effects).
pub const MAX_EFFECTS_PER_LIBRARY: usize = 16_384;
/// Default maximum number of resources in one effect.
pub const MAX_RESOURCES_PER_EFFECT: usize = 512;
/// Maximum authored name length for surfaces, effects, libraries and rows.
pub const MAX_NAME_BYTES: usize = 256;
/// Maximum authored resource-reference length.
pub const MAX_REFERENCE_BYTES: usize = 1_024;

/// Parser bounds. Tests and callers that inspect an unusually large build can
/// tighten (or, with care, raise) the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialEffectsLimits {
    pub max_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_attributes: usize,
    pub max_surfaces: usize,
    pub max_matrix_rows: usize,
    pub max_matrix_columns: usize,
    pub max_effects: usize,
    pub max_resources_per_effect: usize,
    pub max_effect_depth: usize,
}

impl Default for MaterialEffectsLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_XML_BYTES,
            max_nodes: MAX_XML_NODES,
            max_depth: MAX_XML_DEPTH,
            max_attributes: MAX_XML_ATTRIBUTES,
            max_surfaces: MAX_SURFACE_TYPES,
            max_matrix_rows: MAX_MATRIX_ROWS,
            max_matrix_columns: MAX_MATRIX_COLUMNS,
            max_effects: MAX_EFFECTS_PER_LIBRARY,
            max_resources_per_effect: MAX_RESOURCES_PER_EFFECT,
            max_effect_depth: 16,
        }
    }
}

/// Material-effects parsing failures. A malformed individual matrix effect cell
/// is reported as `MatrixCellValue::Invalid`; structural problems and unsafe
/// names or references fail the whole document.
#[derive(Debug, thiserror::Error)]
pub enum MaterialEffectsError {
    #[error("Material-effects input is too large or too deeply nested")]
    Unsafe,
    #[error("Malformed material-effects XML: {0}")]
    Xml(String),
    #[error("Malformed material-effects document: {0}")]
    Malformed(String),
    #[error("Unsafe material-effects name or reference: {0}")]
    UnsafePath(String),
    #[error("Material-effects I/O: {0}")]
    Io(#[from] io::Error),
}

/// A non-fatal schema observation. Unknown attributes are also preserved in the
/// owning `AttributeSet`; this list makes schema drift visible to the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub context: String,
    pub message: String,
}

/// One XML attribute, with its authored qualified name and unescaped value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XmlAttribute {
    pub name: String,
    pub value: String,
}

/// Attribute order as authored. Unknown attributes are retained, not discarded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeSet {
    pub attributes: Vec<XmlAttribute>,
}

impl AttributeSet {
    /// Case-insensitive lookup by local name (`ss:Index` also answers
    /// `Index`). Values are returned exactly as authored.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| local_name(&attribute.name).eq_ignore_ascii_case(name))
            .map(|attribute| attribute.value.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }
}

/// An element the parser does not model. Its text and attributes are preserved
/// so a new schema attribute is visible instead of silently dropped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmodeledElement {
    pub name: String,
    pub attributes: AttributeSet,
    pub text: String,
}

/// Known `ESurfaceTypeFlags` bits, derived only from explicit authored
/// attributes. Missing attributes are false, not engine defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceFlags {
    pub no_physicalize: bool,
    pub no_collide: bool,
    pub vehicle_only_collision: bool,
    pub can_shatter: bool,
    pub bullet_pierceable: bool,
}

/// One `<BreakageParticles>` entry. `effect` comes from the authored `effect`
/// attribute or, when absent, `particle_effect`; `effect_attribute` records
/// which spelling was used. The full attribute set is preserved either way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakageParticle {
    pub breakage_type: Option<String>,
    pub effect: Option<String>,
    pub effect_attribute: Option<String>,
    pub attributes: AttributeSet,
}

/// One authored surface type. `physics`, `ai` and `breakable_2d` are `None`
/// when the element is absent; present elements keep every attribute verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceType {
    pub name: String,
    pub type_name: Option<String>,
    pub attributes: AttributeSet,
    pub physics: Option<AttributeSet>,
    pub ai: Option<AttributeSet>,
    pub breakable_2d: Option<AttributeSet>,
    pub breakage_particles: Vec<BreakageParticle>,
    pub flags: SurfaceFlags,
    pub unknown_elements: Vec<UnmodeledElement>,
}

impl SurfaceType {
    pub fn physics_attribute(&self, name: &str) -> Option<&str> {
        self.physics
            .as_ref()
            .and_then(|attributes| attributes.get(name))
    }

    pub fn ai_attribute(&self, name: &str) -> Option<&str> {
        self.ai.as_ref().and_then(|attributes| attributes.get(name))
    }

    pub fn breakable_2d_attribute(&self, name: &str) -> Option<&str> {
        self.breakable_2d
            .as_ref()
            .and_then(|attributes| attributes.get(name))
    }
}

/// Result of `SurfaceTypeCatalog::breakage_particles`. `Default` is only
/// returned when the caller explicitly requests the engine's default-surface
/// fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BreakageLookup<'a> {
    Specific(&'a BreakageParticle),
    Default(&'a BreakageParticle),
    Missing(&'a SurfaceType),
    NotDefined,
}

/// Parsed `SurfaceTypes.xml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTypeCatalog {
    pub surfaces: Vec<SurfaceType>,
    pub unknown_elements: Vec<UnmodeledElement>,
    pub diagnostics: Vec<Diagnostic>,
}

impl SurfaceTypeCatalog {
    /// Case-insensitive exact lookup. No prefix, suffix or fuzzy matching is
    /// performed.
    pub fn get(&self, name: &str) -> Option<&SurfaceType> {
        self.surfaces
            .iter()
            .find(|surface| surface.name.eq_ignore_ascii_case(name))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.surfaces.iter().map(|surface| surface.name.as_str())
    }

    /// Mirrors `ISurfaceType::GetBreakageParticles(type, lookInDefault)`.
    /// The specific entry always wins. With `look_in_default`, a missing entry
    /// on a non-default surface falls back to `mat_default`; the fallback is
    /// reported as `Default` so callers cannot mistake it for a specific entry.
    pub fn breakage_particles(
        &self,
        surface_name: &str,
        breakage_type: &str,
        look_in_default: bool,
    ) -> BreakageLookup<'_> {
        let Some(surface) = self.get(surface_name) else {
            return BreakageLookup::NotDefined;
        };
        if breakage_type.is_empty() {
            return BreakageLookup::Missing(surface);
        }
        if let Some(particle) = specific_particle(surface, breakage_type) {
            return BreakageLookup::Specific(particle);
        }
        if look_in_default
            && !surface.name.eq_ignore_ascii_case("mat_default")
            && let Some(default) = self.get("mat_default")
            && let Some(particle) = specific_particle(default, breakage_type)
        {
            return BreakageLookup::Default(particle);
        }
        BreakageLookup::Missing(surface)
    }
}

fn specific_particle<'a>(
    surface: &'a SurfaceType,
    breakage_type: &str,
) -> Option<&'a BreakageParticle> {
    surface.breakage_particles.iter().find(|particle| {
        particle
            .breakage_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case(breakage_type))
    })
}

/// One workbook column. `index` is the zero-based XML cell column, including
/// the unnamed leading column. Blank header cells are retained so sparse
/// positions map exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixColumn {
    pub index: usize,
    pub name: String,
}

/// Row role in the workbook. `Custom` rows are authored query names such as
/// `PistolBullet` or `Breakage:Breakage`; `Blank` rows have no first-column name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatrixRowKind {
    Surface,
    Custom,
    Blank,
}

/// A parsed `lib:effect` reference. `raw` retains the authored cell text; the
/// split fields are trimmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectReference {
    pub raw: String,
    pub library: String,
    pub effect: String,
}

/// A non-empty matrix cell that is not a valid `lib:effect` reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidEffectCell {
    pub raw: String,
    pub reason: String,
}

/// Value of an authored matrix cell. `Blank` is an explicit empty `<Cell/>`;
/// a cell absent from `MatrixRow::cells` is missing instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatrixCellValue {
    Blank,
    Effect(EffectReference),
    Invalid(InvalidEffectCell),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixEntry {
    pub column: usize,
    pub value: MatrixCellValue,
}

/// One data row. `cells` excludes the first-column row name and is ordered by
/// authored column position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixRow {
    pub index: usize,
    pub name: String,
    pub kind: MatrixRowKind,
    pub cells: Vec<MatrixEntry>,
}

impl MatrixRow {
    pub fn cell(&self, column: usize) -> Option<&MatrixCellValue> {
        self.cells
            .iter()
            .find(|entry| entry.column == column)
            .map(|entry| &entry.value)
    }
}

/// Result of looking up one matrix cell. Missing and blank remain distinct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatrixLookup<'a> {
    NotDefined {
        row: &'a str,
        column: &'a str,
        row_found: bool,
        column_found: bool,
    },
    Missing {
        row: &'a MatrixRow,
        column: &'a MatrixColumn,
    },
    Blank {
        row: &'a MatrixRow,
        column: &'a MatrixColumn,
    },
    Effect {
        row: &'a MatrixRow,
        column: &'a MatrixColumn,
        reference: &'a EffectReference,
    },
    Invalid {
        row: &'a MatrixRow,
        column: &'a MatrixColumn,
        cell: &'a InvalidEffectCell,
    },
}

/// Whether a surface has a workbook row and/or column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceMatrixCoverage {
    pub surface: String,
    pub has_row: bool,
    pub has_column: bool,
}

/// Parsed Excel-2003 `MaterialEffects.xml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectMatrix {
    pub columns: Vec<MatrixColumn>,
    pub rows: Vec<MatrixRow>,
    pub declared_columns: Option<usize>,
    pub declared_rows: Option<usize>,
    pub unknown_elements: Vec<UnmodeledElement>,
    pub diagnostics: Vec<Diagnostic>,
}

impl EffectMatrix {
    pub fn column(&self, name: &str) -> Option<&MatrixColumn> {
        self.columns
            .iter()
            .find(|column| !column.name.is_empty() && column.name.eq_ignore_ascii_case(name))
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.column(name).map(|column| column.index)
    }

    pub fn row(&self, name: &str) -> Option<&MatrixRow> {
        self.rows
            .iter()
            .find(|row| !row.name.is_empty() && row.name.eq_ignore_ascii_case(name))
    }

    pub fn row_index(&self, name: &str) -> Option<usize> {
        self.row(name).map(|row| row.index)
    }

    /// Directional lookup: `row_name` is the authored row (first column) and
    /// `column_name` is the authored header. No transposition or default
    /// fallback is applied.
    pub fn cell<'a>(&'a self, row_name: &'a str, column_name: &'a str) -> MatrixLookup<'a> {
        let row = self.row(row_name);
        let column = self.column(column_name);
        let (Some(row), Some(column)) = (row, column) else {
            return MatrixLookup::NotDefined {
                row: row_name,
                column: column_name,
                row_found: row.is_some(),
                column_found: column.is_some(),
            };
        };
        match row.cell(column.index) {
            Some(MatrixCellValue::Effect(reference)) => MatrixLookup::Effect {
                row,
                column,
                reference,
            },
            Some(MatrixCellValue::Blank) => MatrixLookup::Blank { row, column },
            Some(MatrixCellValue::Invalid(cell)) => MatrixLookup::Invalid { row, column, cell },
            None => MatrixLookup::Missing { row, column },
        }
    }

    /// Alias for a surface pair lookup. The first surface is the row.
    pub fn lookup_pair<'a>(&'a self, surface_a: &'a str, surface_b: &'a str) -> MatrixLookup<'a> {
        self.cell(surface_a, surface_b)
    }

    /// Report row/column presence for every defined surface.
    pub fn surface_coverage(&self, surfaces: &SurfaceTypeCatalog) -> Vec<SurfaceMatrixCoverage> {
        surfaces
            .surfaces
            .iter()
            .map(|surface| SurfaceMatrixCoverage {
                surface: surface.name.clone(),
                has_row: self.row(&surface.name).is_some(),
                has_column: self.column(&surface.name).is_some(),
            })
            .collect()
    }

    /// Surfaces with neither a row nor a column in this workbook.
    pub fn surfaces_without_row_or_column(&self, surfaces: &SurfaceTypeCatalog) -> Vec<String> {
        self.surface_coverage(surfaces)
            .into_iter()
            .filter(|coverage| !coverage.has_row && !coverage.has_column)
            .map(|coverage| coverage.surface)
            .collect()
    }
}

/// Resource kinds understood by the Epic `MFX_PLAY_*` flags. `DecalTexture` and
/// `DecalMaterial` are kept separate because one is a texture path and the
/// other is an `.mtl` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectResourceKind {
    Sound,
    Particle,
    DecalTexture,
    DecalMaterial,
    FlowGraph,
    Unknown,
}

/// One resolved-by-name resource reference. It is not opened or executed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectResource {
    pub kind: EffectResourceKind,
    pub reference: String,
    /// Attributes of the element that carries the reference (for example the
    /// `Name` scales or the `Decal` lifetime), not of the parent `Effect`.
    pub attributes: AttributeSet,
    /// Enclosing `<RandEffect name="...">`, when the resource was authored in a
    /// random-effect group.
    pub group: Option<String>,
    /// Authored element path, e.g. `Sound/Name` or `Decal/Material`.
    pub element: String,
}

/// A parsed `<Effect>` definition. Inheritance via `parent` is preserved but is
/// not expanded: this module never invents resources from a parent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectDefinition {
    pub name: String,
    pub parent: Option<String>,
    pub delay: Option<String>,
    pub max_distance: Option<String>,
    pub attributes: AttributeSet,
    pub resources: Vec<EffectResource>,
    pub nested_effects: Vec<EffectDefinition>,
    pub unknown_elements: Vec<UnmodeledElement>,
}

impl EffectDefinition {
    /// Direct resource references (nested effects are separate).
    pub fn resource_references(&self) -> impl Iterator<Item = &str> {
        self.resources
            .iter()
            .map(|resource| resource.reference.as_str())
    }
}

/// Parsed one `FXLibs/*.xml` library.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectLibrary {
    pub name: String,
    pub effects: Vec<EffectDefinition>,
    pub unknown_elements: Vec<UnmodeledElement>,
    pub diagnostics: Vec<Diagnostic>,
}

impl EffectLibrary {
    /// Case-insensitive exact effect lookup. Nested effects are not searched.
    pub fn effect(&self, name: &str) -> Option<&EffectDefinition> {
        self.effects
            .iter()
            .find(|effect| effect.name.eq_ignore_ascii_case(name))
    }

    pub fn effect_names(&self) -> impl Iterator<Item = &str> {
        self.effects.iter().map(|effect| effect.name.as_str())
    }
}

/// Result of resolving a `lib:effect` reference against parsed FXLibs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectLookup<'a> {
    LibraryNotDefined(&'a str),
    EffectNotDefined { library: &'a str, effect: &'a str },
    Found(&'a EffectDefinition),
}

/// Convenience container for a parsed SDK `Libs/MaterialEffects` set. It does
/// not read resources from disk and does not execute anything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEffectsCatalog {
    pub surfaces: SurfaceTypeCatalog,
    pub matrix: EffectMatrix,
    pub libraries: Vec<EffectLibrary>,
}

impl MaterialEffectsCatalog {
    pub fn new(
        surfaces: SurfaceTypeCatalog,
        matrix: EffectMatrix,
        libraries: Vec<EffectLibrary>,
    ) -> Self {
        Self {
            surfaces,
            matrix,
            libraries,
        }
    }

    pub fn library(&self, name: &str) -> Option<&EffectLibrary> {
        self.libraries
            .iter()
            .find(|library| library.name.eq_ignore_ascii_case(name))
    }

    pub fn effect_by_name<'a>(&'a self, library: &'a str, effect: &'a str) -> EffectLookup<'a> {
        let Some(library) = self.library(library) else {
            return EffectLookup::LibraryNotDefined(library);
        };
        match library.effect(effect) {
            Some(effect) => EffectLookup::Found(effect),
            None => EffectLookup::EffectNotDefined {
                library: library.name.as_str(),
                effect,
            },
        }
    }

    pub fn effect<'a>(&'a self, reference: &'a EffectReference) -> EffectLookup<'a> {
        self.effect_by_name(&reference.library, &reference.effect)
    }
}

/// Parse `SurfaceTypes.xml`.
pub fn parse_surface_types(bytes: &[u8]) -> Result<SurfaceTypeCatalog, MaterialEffectsError> {
    parse_surface_types_with_limits(bytes, &MaterialEffectsLimits::default())
}

/// Parse `SurfaceTypes.xml` with explicit bounds.
pub fn parse_surface_types_with_limits(
    bytes: &[u8],
    limits: &MaterialEffectsLimits,
) -> Result<SurfaceTypeCatalog, MaterialEffectsError> {
    let tree = XmlTree::parse(bytes, limits)?;
    let root = tree.root;
    if !local_name(&tree.nodes[root].name).eq_ignore_ascii_case("SurfaceTypes") {
        return Err(MaterialEffectsError::Malformed(
            "SurfaceTypes.xml root must be <SurfaceTypes>".into(),
        ));
    }
    let mut catalog = SurfaceTypeCatalog::default();
    let mut names = HashSet::new();
    for child in tree.children_of(root) {
        let node = &tree.nodes[child];
        if local_name(&node.name).eq_ignore_ascii_case("SurfaceType") {
            if catalog.surfaces.len() >= limits.max_surfaces {
                return Err(MaterialEffectsError::Unsafe);
            }
            let surface = parse_surface_type(&tree, child, &mut catalog.diagnostics)?;
            if !names.insert(surface.name.to_ascii_lowercase()) {
                catalog.diagnostics.push(Diagnostic {
                    context: format!("SurfaceType {}", surface.name),
                    message: "duplicate surface name; lookup uses the first entry".into(),
                });
            }
            catalog.surfaces.push(surface);
        } else {
            catalog.unknown_elements.push(unmodeled(&tree, child));
            catalog.diagnostics.push(Diagnostic {
                context: "SurfaceTypes".into(),
                message: format!("unmodeled element <{}> preserved", node.name),
            });
        }
    }
    report_unexpected_text(&mut catalog.diagnostics, "SurfaceTypes", &tree.nodes[root]);
    Ok(catalog)
}

fn parse_surface_type(
    tree: &XmlTree,
    index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<SurfaceType, MaterialEffectsError> {
    let node = &tree.nodes[index];
    let name = node
        .attributes
        .get("name")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            MaterialEffectsError::Malformed("SurfaceType is missing a non-empty name".into())
        })?;
    validate_identifier("SurfaceType name", name, false)?;
    let type_name = node
        .attributes
        .get("type")
        .map(|value| value.trim().to_owned());
    if let Some(type_name) = type_name.as_deref().filter(|value| !value.is_empty()) {
        validate_identifier("SurfaceType type", type_name, false)?;
    }
    let context = format!("SurfaceType {name}");
    report_unknown_attributes(diagnostics, &context, &node.attributes, &["name", "type"]);

    let mut physics: Option<AttributeSet> = None;
    let mut ai: Option<AttributeSet> = None;
    let mut breakable_2d: Option<AttributeSet> = None;
    let mut breakage_particles = Vec::new();
    let mut unknown_elements = Vec::new();

    for child in tree.children_of(index) {
        let child_node = &tree.nodes[child];
        match local_name(&child_node.name).to_ascii_lowercase().as_str() {
            "physics" => {
                if physics.is_some() {
                    diagnostics.push(Diagnostic {
                        context: context.clone(),
                        message: "duplicate <Physics>; first entry kept".into(),
                    });
                } else {
                    report_unknown_attributes(
                        diagnostics,
                        &format!("{context} / Physics"),
                        &child_node.attributes,
                        PHYSICS_ATTRIBUTES,
                    );
                    physics = Some(child_node.attributes.clone());
                }
                report_unexpected_text(diagnostics, &context, child_node);
            }
            "ai" => {
                if ai.is_some() {
                    diagnostics.push(Diagnostic {
                        context: context.clone(),
                        message: "duplicate <AI>; first entry kept".into(),
                    });
                } else {
                    report_unknown_attributes(
                        diagnostics,
                        &format!("{context} / AI"),
                        &child_node.attributes,
                        AI_ATTRIBUTES,
                    );
                    ai = Some(child_node.attributes.clone());
                }
                report_unexpected_text(diagnostics, &context, child_node);
            }
            "breakable_2d" => {
                if breakable_2d.is_some() {
                    diagnostics.push(Diagnostic {
                        context: context.clone(),
                        message: "duplicate <breakable_2d>; first entry kept".into(),
                    });
                } else {
                    report_unknown_attributes(
                        diagnostics,
                        &format!("{context} / breakable_2d"),
                        &child_node.attributes,
                        BREAKABLE_2D_ATTRIBUTES,
                    );
                    breakable_2d = Some(child_node.attributes.clone());
                }
                report_unexpected_text(diagnostics, &context, child_node);
            }
            "breakageparticles" => {
                breakage_particles.push(parse_breakage_particle(
                    tree,
                    child,
                    &context,
                    diagnostics,
                    &mut unknown_elements,
                )?);
            }
            _ => {
                unknown_elements.push(unmodeled(tree, child));
                diagnostics.push(Diagnostic {
                    context: context.clone(),
                    message: format!("unmodeled element <{}> preserved", child_node.name),
                });
            }
        }
    }
    report_unexpected_text(diagnostics, &context, node);

    let flags = physics
        .as_ref()
        .map(|attributes| SurfaceFlags::from_attributes(attributes, diagnostics, &context))
        .unwrap_or_default();
    Ok(SurfaceType {
        name: name.to_owned(),
        type_name,
        attributes: node.attributes.clone(),
        physics,
        ai,
        breakable_2d,
        breakage_particles,
        flags,
        unknown_elements,
    })
}

impl SurfaceFlags {
    fn from_attributes(
        attributes: &AttributeSet,
        diagnostics: &mut Vec<Diagnostic>,
        context: &str,
    ) -> Self {
        Self {
            no_physicalize: authored_flag(attributes, "no_physicalize", diagnostics, context),
            no_collide: authored_flag(attributes, "no_collide", diagnostics, context),
            vehicle_only_collision: authored_flag(
                attributes,
                "vehicle_only_collision",
                diagnostics,
                context,
            ),
            can_shatter: authored_flag(attributes, "can_shatter", diagnostics, context),
            bullet_pierceable: authored_flag(attributes, "bullet_pierceable", diagnostics, context),
        }
    }
}

fn authored_flag(
    attributes: &AttributeSet,
    name: &str,
    diagnostics: &mut Vec<Diagnostic>,
    context: &str,
) -> bool {
    let Some(raw) = attributes.get(name) else {
        return false;
    };
    let value = raw.trim();
    if value == "1" || value.eq_ignore_ascii_case("true") {
        true
    } else if value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false") {
        false
    } else {
        diagnostics.push(Diagnostic {
            context: context.to_owned(),
            message: format!("{name} has unsupported boolean value '{raw}'; treated as false"),
        });
        false
    }
}

fn parse_breakage_particle(
    tree: &XmlTree,
    index: usize,
    surface_context: &str,
    diagnostics: &mut Vec<Diagnostic>,
    unknown_elements: &mut Vec<UnmodeledElement>,
) -> Result<BreakageParticle, MaterialEffectsError> {
    let node = &tree.nodes[index];
    let context = format!("{surface_context} / BreakageParticles");
    report_unknown_attributes(
        diagnostics,
        &context,
        &node.attributes,
        BREAKAGE_PARTICLE_ATTRIBUTES,
    );
    let breakage_type = node
        .attributes
        .get("type")
        .map(|value| value.trim().to_owned());
    if let Some(kind) = breakage_type.as_deref().filter(|value| !value.is_empty()) {
        validate_identifier("BreakageParticles type", kind, false)?;
    }
    let effect = node
        .attributes
        .get("effect")
        .or_else(|| node.attributes.get("particle_effect"))
        .map(|value| value.trim().to_owned());
    let effect_attribute = if node.attributes.get("effect").is_some() {
        Some("effect".to_owned())
    } else if node.attributes.get("particle_effect").is_some() {
        Some("particle_effect".to_owned())
    } else {
        None
    };
    if node.attributes.get("effect").is_some() && node.attributes.get("particle_effect").is_some() {
        diagnostics.push(Diagnostic {
            context: context.clone(),
            message: "both effect and particle_effect are authored; effect is used".into(),
        });
    }
    if let Some(effect) = effect.as_deref().filter(|value| !value.is_empty()) {
        validate_reference("BreakageParticles effect", effect)?;
    }
    for child in tree.children_of(index) {
        unknown_elements.push(unmodeled(tree, child));
        diagnostics.push(Diagnostic {
            context: context.clone(),
            message: format!(
                "unmodeled child <{}> of BreakageParticles preserved",
                tree.nodes[child].name
            ),
        });
    }
    Ok(BreakageParticle {
        breakage_type,
        effect,
        effect_attribute,
        attributes: node.attributes.clone(),
    })
}

/// Parse the Excel-2003 `MaterialEffects.xml` workbook.
pub fn parse_effect_matrix(bytes: &[u8]) -> Result<EffectMatrix, MaterialEffectsError> {
    parse_effect_matrix_with_limits(bytes, &MaterialEffectsLimits::default())
}

/// Parse the workbook with explicit bounds.
pub fn parse_effect_matrix_with_limits(
    bytes: &[u8],
    limits: &MaterialEffectsLimits,
) -> Result<EffectMatrix, MaterialEffectsError> {
    let tree = XmlTree::parse(bytes, limits)?;
    let root = tree.root;
    if !local_name(&tree.nodes[root].name).eq_ignore_ascii_case("Workbook") {
        return Err(MaterialEffectsError::Malformed(
            "MaterialEffects.xml root must be <Workbook>".into(),
        ));
    }
    let table = find_first_element(&tree, root, "Table").ok_or_else(|| {
        MaterialEffectsError::Malformed("MaterialEffects.xml has no worksheet <Table>".into())
    })?;
    let table_node = &tree.nodes[table];
    let declared_columns = optional_usize(
        &table_node.attributes,
        "ExpandedColumnCount",
        "ExpandedColumnCount",
    )?;
    let declared_rows = optional_usize(
        &table_node.attributes,
        "ExpandedRowCount",
        "ExpandedRowCount",
    )?;
    if declared_columns.is_some_and(|count| count > limits.max_matrix_columns)
        || declared_rows.is_some_and(|count| count > limits.max_matrix_rows)
    {
        return Err(MaterialEffectsError::Unsafe);
    }

    let mut matrix = EffectMatrix {
        declared_columns,
        declared_rows,
        ..EffectMatrix::default()
    };
    let mut authored_rows = Vec::new();
    let mut next_row_index = 0usize;
    for child in tree.children_of(table) {
        let child_node = &tree.nodes[child];
        match local_name(&child_node.name).to_ascii_lowercase().as_str() {
            "row" => {
                let index = match child_node.attributes.get("Index") {
                    Some(raw) => parse_one_based(raw, "row ss:Index")?,
                    None => next_row_index,
                };
                if index >= limits.max_matrix_rows {
                    return Err(MaterialEffectsError::Unsafe);
                }
                next_row_index = index.saturating_add(1);
                authored_rows.push((index, child));
            }
            "column" => {}
            _ => {
                matrix.unknown_elements.push(unmodeled(&tree, child));
                matrix.diagnostics.push(Diagnostic {
                    context: "Table".into(),
                    message: format!("unmodeled table element <{}> preserved", child_node.name),
                });
            }
        }
    }
    let Some((_, header_node)) = authored_rows.first().copied() else {
        return Err(MaterialEffectsError::Malformed(
            "MaterialEffects.xml table has no header row".into(),
        ));
    };
    let header_cells = parse_cells(
        &tree,
        header_node,
        limits,
        &mut matrix.diagnostics,
        "header row",
    )?;
    if header_cells.is_empty() {
        return Err(MaterialEffectsError::Malformed(
            "MaterialEffects.xml header row has no cells".into(),
        ));
    }
    if header_cells
        .iter()
        .any(|cell| cell.column == 0 && !cell.blank)
    {
        matrix.diagnostics.push(Diagnostic {
            context: "header row".into(),
            message: "column 0 is normally unnamed; its value is preserved as a column".into(),
        });
    }
    for cell in &header_cells {
        matrix.columns.push(MatrixColumn {
            index: cell.column,
            name: cell.text.trim().to_owned(),
        });
    }
    let mut column_names = HashSet::new();
    for column in &matrix.columns {
        if !column.name.is_empty() && !column_names.insert(column.name.to_ascii_lowercase()) {
            matrix.diagnostics.push(Diagnostic {
                context: "header row".into(),
                message: format!(
                    "duplicate column '{}'; lookup uses the first entry",
                    column.name
                ),
            });
        }
    }

    let mut row_names = HashSet::new();
    for (index, row_node) in authored_rows.into_iter().skip(1) {
        let cells = parse_cells(
            &tree,
            row_node,
            limits,
            &mut matrix.diagnostics,
            &format!("row {}", index + 1),
        )?;
        let name = cells
            .iter()
            .find(|cell| cell.column == 0)
            .map(|cell| cell.text.trim().to_owned())
            .unwrap_or_default();
        if !name.is_empty() {
            validate_identifier("matrix row name", &name, true)?;
            if !row_names.insert(name.to_ascii_lowercase()) {
                matrix.diagnostics.push(Diagnostic {
                    context: format!("matrix row {}", index + 1),
                    message: format!("duplicate row '{name}'; lookup uses the first entry"),
                });
            }
        }
        let kind = if name.is_empty() {
            MatrixRowKind::Blank
        } else if matrix
            .columns
            .iter()
            .any(|column| !column.name.is_empty() && column.name.eq_ignore_ascii_case(&name))
        {
            MatrixRowKind::Surface
        } else {
            MatrixRowKind::Custom
        };
        let mut entries = Vec::new();
        for cell in cells.into_iter().filter(|cell| cell.column != 0) {
            let value = if cell.blank {
                MatrixCellValue::Blank
            } else {
                parse_effect_cell(
                    &cell.text,
                    &format!("matrix row {} / column {}", index + 1, cell.column + 1),
                    &mut matrix.diagnostics,
                )
            };
            entries.push(MatrixEntry {
                column: cell.column,
                value,
            });
        }
        matrix.rows.push(MatrixRow {
            index,
            name,
            kind,
            cells: entries,
        });
    }
    Ok(matrix)
}

fn parse_effect_cell(
    raw: &str,
    context: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> MatrixCellValue {
    let value = raw.trim();
    if value.is_empty() {
        return MatrixCellValue::Blank;
    }
    let mut parts = value.split(':');
    let library = parts.next().unwrap_or_default().trim();
    let effect = parts.next().unwrap_or_default().trim();
    let extra = parts.next();
    let reason = if library.is_empty() || effect.is_empty() || extra.is_some() {
        "expected exactly one non-empty ':' in lib:effect".to_owned()
    } else {
        match validate_identifier("effect library", library, false)
            .and_then(|()| validate_identifier("effect name", effect, false))
        {
            Ok(()) => {
                return MatrixCellValue::Effect(EffectReference {
                    raw: raw.to_owned(),
                    library: library.to_owned(),
                    effect: effect.to_owned(),
                });
            }
            Err(error) => error.to_string(),
        }
    };
    diagnostics.push(Diagnostic {
        context: context.to_owned(),
        message: format!("invalid effect cell '{raw}': {reason}"),
    });
    MatrixCellValue::Invalid(InvalidEffectCell {
        raw: raw.to_owned(),
        reason,
    })
}

#[derive(Clone, Debug)]
struct RawCell {
    column: usize,
    text: String,
    blank: bool,
}

fn parse_cells(
    tree: &XmlTree,
    row: usize,
    limits: &MaterialEffectsLimits,
    diagnostics: &mut Vec<Diagnostic>,
    context: &str,
) -> Result<Vec<RawCell>, MaterialEffectsError> {
    let mut cells = Vec::new();
    let mut cursor = 0usize;
    for child in tree.children_of(row) {
        let node = &tree.nodes[child];
        if !local_name(&node.name).eq_ignore_ascii_case("Cell") {
            if !node.text.trim().is_empty() {
                diagnostics.push(Diagnostic {
                    context: context.to_owned(),
                    message: format!("unexpected text inside <{}>", node.name),
                });
            }
            continue;
        }
        let column = match node.attributes.get("Index") {
            Some(raw) => parse_one_based(raw, "cell ss:Index")?,
            None => cursor,
        };
        if column < cursor {
            return Err(MaterialEffectsError::Malformed(
                "out-of-order cell ss:Index".into(),
            ));
        }
        if column >= limits.max_matrix_columns {
            return Err(MaterialEffectsError::Unsafe);
        }
        let span = optional_count(&node.attributes, "Span", "cell ss:Span")?;
        let merge_across = optional_count(&node.attributes, "MergeAcross", "cell ss:MergeAcross")?;
        let merge_down = optional_count(&node.attributes, "MergeDown", "cell ss:MergeDown")?;
        let advance = 1usize
            .checked_add(span)
            .and_then(|value| value.checked_add(merge_across))
            .ok_or(MaterialEffectsError::Unsafe)?;
        let next = column
            .checked_add(advance)
            .ok_or(MaterialEffectsError::Unsafe)?;
        if next > limits.max_matrix_columns {
            return Err(MaterialEffectsError::Unsafe);
        }
        if merge_down > 0 {
            diagnostics.push(Diagnostic {
                context: context.to_owned(),
                message: format!(
                    "cell at column {} has ss:MergeDown={merge_down}; vertical merges are not expanded",
                    column + 1
                ),
            });
        }
        let (has_data, text) = cell_data(tree, child);
        cells.push(RawCell {
            column,
            blank: !has_data || text.trim().is_empty(),
            text,
        });
        cursor = next;
    }
    Ok(cells)
}

fn cell_data(tree: &XmlTree, cell: usize) -> (bool, String) {
    let mut has_data = false;
    let mut text = String::new();
    for child in tree.children_of(cell) {
        let node = &tree.nodes[child];
        if local_name(&node.name).eq_ignore_ascii_case("Data") {
            has_data = true;
            text.push_str(&node.text);
        }
    }
    (has_data, text)
}

fn optional_count(
    attributes: &AttributeSet,
    name: &str,
    label: &str,
) -> Result<usize, MaterialEffectsError> {
    match attributes.get(name) {
        Some(raw) => raw.trim().parse::<usize>().map_err(|_| {
            MaterialEffectsError::Malformed(format!("{label} is not a non-negative integer"))
        }),
        None => Ok(0),
    }
}

fn optional_usize(
    attributes: &AttributeSet,
    name: &str,
    label: &str,
) -> Result<Option<usize>, MaterialEffectsError> {
    match attributes.get(name) {
        Some(raw) if !raw.trim().is_empty() => {
            raw.trim().parse::<usize>().map(Some).map_err(|_| {
                MaterialEffectsError::Malformed(format!("{label} is not a non-negative integer"))
            })
        }
        _ => Ok(None),
    }
}

fn parse_one_based(raw: &str, label: &str) -> Result<usize, MaterialEffectsError> {
    let value = raw
        .trim()
        .parse::<usize>()
        .map_err(|_| MaterialEffectsError::Malformed(format!("{label} is not an integer")))?;
    value
        .checked_sub(1)
        .ok_or_else(|| MaterialEffectsError::Malformed(format!("{label} is 1-based")))
}

/// Parse one `FXLibs/*.xml` library. `name` is the file stem, e.g. `collisions`.
pub fn parse_fx_library(name: &str, bytes: &[u8]) -> Result<EffectLibrary, MaterialEffectsError> {
    parse_fx_library_with_limits(name, bytes, &MaterialEffectsLimits::default())
}

/// Parse one FXLib with explicit bounds.
pub fn parse_fx_library_with_limits(
    name: &str,
    bytes: &[u8],
    limits: &MaterialEffectsLimits,
) -> Result<EffectLibrary, MaterialEffectsError> {
    validate_identifier("FXLib name", name, false)?;
    let tree = XmlTree::parse(bytes, limits)?;
    let root = tree.root;
    if !local_name(&tree.nodes[root].name).eq_ignore_ascii_case("FXLib") {
        return Err(MaterialEffectsError::Malformed(
            "FXLib root must be <FXLib>".into(),
        ));
    }
    let mut library = EffectLibrary {
        name: name.to_owned(),
        ..EffectLibrary::default()
    };
    report_unknown_attributes(
        &mut library.diagnostics,
        &format!("FXLib {name}"),
        &tree.nodes[root].attributes,
        &[],
    );
    let mut effect_count = 0usize;
    let mut effect_names = HashSet::new();
    for child in tree.children_of(root) {
        let node = &tree.nodes[child];
        if local_name(&node.name).eq_ignore_ascii_case("Effect") {
            let effect = parse_effect(
                &tree,
                child,
                limits,
                &mut library.diagnostics,
                &mut effect_count,
                0,
            )?;
            if !effect_names.insert(effect.name.to_ascii_lowercase()) {
                library.diagnostics.push(Diagnostic {
                    context: format!("FXLib {name}"),
                    message: format!(
                        "duplicate effect '{}'; lookup uses the first entry",
                        effect.name
                    ),
                });
            }
            library.effects.push(effect);
        } else {
            library.unknown_elements.push(unmodeled(&tree, child));
            library.diagnostics.push(Diagnostic {
                context: format!("FXLib {name}"),
                message: format!("unmodeled element <{}> preserved", node.name),
            });
        }
    }
    report_unexpected_text(
        &mut library.diagnostics,
        &format!("FXLib {name}"),
        &tree.nodes[root],
    );
    Ok(library)
}

fn parse_effect(
    tree: &XmlTree,
    index: usize,
    limits: &MaterialEffectsLimits,
    diagnostics: &mut Vec<Diagnostic>,
    effect_count: &mut usize,
    depth: usize,
) -> Result<EffectDefinition, MaterialEffectsError> {
    if depth > limits.max_effect_depth {
        return Err(MaterialEffectsError::Unsafe);
    }
    *effect_count = effect_count.saturating_add(1);
    if *effect_count > limits.max_effects {
        return Err(MaterialEffectsError::Unsafe);
    }
    let node = &tree.nodes[index];
    let name = node
        .attributes
        .get("name")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| MaterialEffectsError::Malformed("Effect is missing a name".into()))?;
    validate_identifier("Effect name", name, false)?;
    let parent = node
        .attributes
        .get("parent")
        .map(str::trim)
        .filter(|parent| !parent.is_empty())
        .map(str::to_owned);
    if let Some(parent) = parent.as_deref() {
        validate_identifier("Effect parent", parent, false)?;
    }
    let context = format!("Effect {name}");
    report_unknown_attributes(
        diagnostics,
        &context,
        &node.attributes,
        &["name", "parent", "delay", "maxDist"],
    );

    let mut definition = EffectDefinition {
        name: name.to_owned(),
        parent,
        delay: node.attributes.get("delay").map(str::to_owned),
        max_distance: node.attributes.get("maxDist").map(str::to_owned),
        attributes: node.attributes.clone(),
        ..EffectDefinition::default()
    };
    let mut unknown_elements = Vec::new();
    for child in tree.children_of(index) {
        let child_node = &tree.nodes[child];
        match local_name(&child_node.name).to_ascii_lowercase().as_str() {
            "sound" => parse_sound(
                tree,
                child,
                None,
                &context,
                limits,
                diagnostics,
                &mut definition.resources,
                &mut unknown_elements,
            )?,
            "particle" => parse_particle(
                tree,
                child,
                None,
                &context,
                limits,
                diagnostics,
                &mut definition.resources,
                &mut unknown_elements,
            )?,
            "decal" => parse_decal(
                tree,
                child,
                None,
                &context,
                limits,
                diagnostics,
                &mut definition.resources,
                &mut unknown_elements,
            )?,
            "flowgraph" => parse_flowgraph(
                tree,
                child,
                None,
                &context,
                limits,
                diagnostics,
                &mut definition.resources,
                &mut unknown_elements,
            )?,
            "randeffect" => parse_rand_effect(
                tree,
                child,
                limits,
                diagnostics,
                effect_count,
                depth,
                &context,
                &mut definition.resources,
                &mut definition.nested_effects,
                &mut unknown_elements,
            )?,
            "effect" => {
                definition.nested_effects.push(parse_effect(
                    tree,
                    child,
                    limits,
                    diagnostics,
                    effect_count,
                    depth + 1,
                )?);
            }
            _ => {
                unknown_elements.push(unmodeled(tree, child));
                diagnostics.push(Diagnostic {
                    context: context.clone(),
                    message: format!("unmodeled element <{}> preserved", child_node.name),
                });
            }
        }
    }
    report_unexpected_text(diagnostics, &context, node);
    definition.unknown_elements = unknown_elements;
    Ok(definition)
}

#[allow(clippy::too_many_arguments)]
fn parse_rand_effect(
    tree: &XmlTree,
    index: usize,
    limits: &MaterialEffectsLimits,
    diagnostics: &mut Vec<Diagnostic>,
    effect_count: &mut usize,
    depth: usize,
    parent_context: &str,
    resources: &mut Vec<EffectResource>,
    nested_effects: &mut Vec<EffectDefinition>,
    unknown_elements: &mut Vec<UnmodeledElement>,
) -> Result<(), MaterialEffectsError> {
    let node = &tree.nodes[index];
    let group = node
        .attributes
        .get("name")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let context = format!("{parent_context} / RandEffect");
    report_unknown_attributes(diagnostics, &context, &node.attributes, &["name"]);
    for child in tree.children_of(index) {
        let child_node = &tree.nodes[child];
        match local_name(&child_node.name).to_ascii_lowercase().as_str() {
            "sound" => parse_sound(
                tree,
                child,
                group.as_deref(),
                &context,
                limits,
                diagnostics,
                resources,
                unknown_elements,
            )?,
            "particle" => parse_particle(
                tree,
                child,
                group.as_deref(),
                &context,
                limits,
                diagnostics,
                resources,
                unknown_elements,
            )?,
            "decal" => parse_decal(
                tree,
                child,
                group.as_deref(),
                &context,
                limits,
                diagnostics,
                resources,
                unknown_elements,
            )?,
            "flowgraph" => parse_flowgraph(
                tree,
                child,
                group.as_deref(),
                &context,
                limits,
                diagnostics,
                resources,
                unknown_elements,
            )?,
            "effect" => nested_effects.push(parse_effect(
                tree,
                child,
                limits,
                diagnostics,
                effect_count,
                depth + 1,
            )?),
            "randeffect" => parse_rand_effect(
                tree,
                child,
                limits,
                diagnostics,
                effect_count,
                depth,
                &context,
                resources,
                nested_effects,
                unknown_elements,
            )?,
            _ => {
                unknown_elements.push(unmodeled(tree, child));
                diagnostics.push(Diagnostic {
                    context: context.clone(),
                    message: format!("unmodeled element <{}> preserved", child_node.name),
                });
            }
        }
    }
    report_unexpected_text(diagnostics, &context, node);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_sound(
    tree: &XmlTree,
    index: usize,
    group: Option<&str>,
    parent_context: &str,
    limits: &MaterialEffectsLimits,
    diagnostics: &mut Vec<Diagnostic>,
    resources: &mut Vec<EffectResource>,
    unknown_elements: &mut Vec<UnmodeledElement>,
) -> Result<(), MaterialEffectsError> {
    let node = &tree.nodes[index];
    let context = format!("{parent_context} / Sound");
    report_unknown_attributes(diagnostics, &context, &node.attributes, &[]);
    for child in tree.children_of(index) {
        let child_node = &tree.nodes[child];
        if local_name(&child_node.name).eq_ignore_ascii_case("Name") {
            let reference = child_node.text.trim();
            if reference.is_empty() {
                diagnostics.push(Diagnostic {
                    context: context.clone(),
                    message: "empty sound name ignored".into(),
                });
                continue;
            }
            validate_reference("Sound name", reference)?;
            report_unknown_attributes(
                diagnostics,
                &format!("{context} / Name"),
                &child_node.attributes,
                &[],
            );
            push_resource(
                resources,
                EffectResource {
                    kind: EffectResourceKind::Sound,
                    reference: reference.to_owned(),
                    attributes: child_node.attributes.clone(),
                    group: group.map(str::to_owned),
                    element: "Sound/Name".into(),
                },
                limits,
            )?;
        } else {
            unknown_elements.push(unmodeled(tree, child));
            diagnostics.push(Diagnostic {
                context: context.clone(),
                message: format!("unmodeled element <{}> preserved", child_node.name),
            });
        }
    }
    report_unexpected_text(diagnostics, parent_context, node);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_particle(
    tree: &XmlTree,
    index: usize,
    group: Option<&str>,
    parent_context: &str,
    limits: &MaterialEffectsLimits,
    diagnostics: &mut Vec<Diagnostic>,
    resources: &mut Vec<EffectResource>,
    unknown_elements: &mut Vec<UnmodeledElement>,
) -> Result<(), MaterialEffectsError> {
    let node = &tree.nodes[index];
    let context = format!("{parent_context} / Particle");
    report_unknown_attributes(diagnostics, &context, &node.attributes, &[]);
    let mut found = false;
    for child in tree.children_of(index) {
        let child_node = &tree.nodes[child];
        if local_name(&child_node.name).eq_ignore_ascii_case("Name") {
            let reference = child_node.text.trim();
            if reference.is_empty() {
                diagnostics.push(Diagnostic {
                    context: context.clone(),
                    message: "empty particle name ignored".into(),
                });
                continue;
            }
            validate_reference("Particle name", reference)?;
            report_unknown_attributes(
                diagnostics,
                &format!("{context} / Name"),
                &child_node.attributes,
                PARTICLE_NAME_ATTRIBUTES,
            );
            found = true;
            push_resource(
                resources,
                EffectResource {
                    kind: EffectResourceKind::Particle,
                    reference: reference.to_owned(),
                    attributes: child_node.attributes.clone(),
                    group: group.map(str::to_owned),
                    element: "Particle/Name".into(),
                },
                limits,
            )?;
        } else {
            unknown_elements.push(unmodeled(tree, child));
            diagnostics.push(Diagnostic {
                context: context.clone(),
                message: format!("unmodeled element <{}> preserved", child_node.name),
            });
        }
    }
    if !found {
        diagnostics.push(Diagnostic {
            context,
            message: "<Particle> has no <Name>".into(),
        });
    }
    report_unexpected_text(diagnostics, parent_context, node);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_decal(
    tree: &XmlTree,
    index: usize,
    group: Option<&str>,
    parent_context: &str,
    limits: &MaterialEffectsLimits,
    diagnostics: &mut Vec<Diagnostic>,
    resources: &mut Vec<EffectResource>,
    unknown_elements: &mut Vec<UnmodeledElement>,
) -> Result<(), MaterialEffectsError> {
    let node = &tree.nodes[index];
    let context = format!("{parent_context} / Decal");
    report_unknown_attributes(diagnostics, &context, &node.attributes, DECAL_ATTRIBUTES);
    let mut found = false;
    for child in tree.children_of(index) {
        let child_node = &tree.nodes[child];
        let (kind, element) = match local_name(&child_node.name).to_ascii_lowercase().as_str() {
            "filename" => (EffectResourceKind::DecalTexture, "Decal/Filename"),
            "material" => (EffectResourceKind::DecalMaterial, "Decal/Material"),
            _ => {
                unknown_elements.push(unmodeled(tree, child));
                diagnostics.push(Diagnostic {
                    context: context.clone(),
                    message: format!("unmodeled element <{}> preserved", child_node.name),
                });
                continue;
            }
        };
        let reference = child_node.text.trim();
        if reference.is_empty() {
            diagnostics.push(Diagnostic {
                context: context.clone(),
                message: format!("empty {element} ignored"),
            });
            continue;
        }
        validate_reference(element, reference)?;
        report_unknown_attributes(
            diagnostics,
            &format!("{context} / {element}"),
            &child_node.attributes,
            &[],
        );
        found = true;
        push_resource(
            resources,
            EffectResource {
                kind,
                reference: reference.to_owned(),
                attributes: node.attributes.clone(),
                group: group.map(str::to_owned),
                element: element.to_owned(),
            },
            limits,
        )?;
    }
    if !found {
        diagnostics.push(Diagnostic {
            context,
            message: "<Decal> has no Filename or Material".into(),
        });
    }
    report_unexpected_text(diagnostics, parent_context, node);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_flowgraph(
    tree: &XmlTree,
    index: usize,
    group: Option<&str>,
    parent_context: &str,
    limits: &MaterialEffectsLimits,
    diagnostics: &mut Vec<Diagnostic>,
    resources: &mut Vec<EffectResource>,
    unknown_elements: &mut Vec<UnmodeledElement>,
) -> Result<(), MaterialEffectsError> {
    let node = &tree.nodes[index];
    let context = format!("{parent_context} / FlowGraph");
    report_unknown_attributes(
        diagnostics,
        &context,
        &node.attributes,
        &["name", "maxdist"],
    );
    let Some(reference) = node
        .attributes
        .get("name")
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        diagnostics.push(Diagnostic {
            context,
            message: "<FlowGraph> has no name; ignored".into(),
        });
        return Ok(());
    };
    validate_reference("FlowGraph name", reference)?;
    push_resource(
        resources,
        EffectResource {
            kind: EffectResourceKind::FlowGraph,
            reference: reference.to_owned(),
            attributes: node.attributes.clone(),
            group: group.map(str::to_owned),
            element: "FlowGraph".into(),
        },
        limits,
    )?;
    for child in tree.children_of(index) {
        unknown_elements.push(unmodeled(tree, child));
        diagnostics.push(Diagnostic {
            context: format!("{parent_context} / FlowGraph"),
            message: format!(
                "unmodeled child <{}> of FlowGraph preserved",
                tree.nodes[child].name
            ),
        });
    }
    Ok(())
}

fn push_resource(
    resources: &mut Vec<EffectResource>,
    resource: EffectResource,
    limits: &MaterialEffectsLimits,
) -> Result<(), MaterialEffectsError> {
    if resources.len() >= limits.max_resources_per_effect {
        return Err(MaterialEffectsError::Unsafe);
    }
    resources.push(resource);
    Ok(())
}

// ---------------------------------------------------------------------------
// Bounded XML tree
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct XmlNode {
    name: String,
    attributes: AttributeSet,
    children: Vec<usize>,
    text: String,
}

#[derive(Clone, Debug)]
struct XmlTree {
    nodes: Vec<XmlNode>,
    root: usize,
}

impl XmlTree {
    fn parse(bytes: &[u8], limits: &MaterialEffectsLimits) -> Result<Self, MaterialEffectsError> {
        if bytes.is_empty() || bytes.len() > limits.max_bytes {
            return Err(MaterialEffectsError::Unsafe);
        }
        let decoded = crate::cryxml::xml_bytes(bytes, limits.max_bytes)
            .map_err(|error| MaterialEffectsError::Xml(error.to_string()))?;
        let mut reader = Reader::from_reader(decoded.as_ref());
        reader.config_mut().trim_text(false);
        reader.config_mut().check_end_names = true;
        let mut tree = Self {
            nodes: Vec::new(),
            root: 0,
        };
        let mut stack: Vec<usize> = Vec::new();
        let mut root = None;
        let mut attribute_count = 0usize;
        loop {
            let event = reader
                .read_event()
                .map_err(|error| MaterialEffectsError::Xml(error.to_string()))?;
            match event {
                Event::Start(element) => {
                    let index = append_node(
                        &mut tree.nodes,
                        &stack,
                        &mut root,
                        &mut attribute_count,
                        &element,
                        &reader,
                        limits,
                    )?;
                    stack.push(index);
                }
                Event::Empty(element) => {
                    append_node(
                        &mut tree.nodes,
                        &stack,
                        &mut root,
                        &mut attribute_count,
                        &element,
                        &reader,
                        limits,
                    )?;
                }
                Event::End(_) => {
                    stack.pop().ok_or_else(|| {
                        MaterialEffectsError::Malformed("unexpected closing tag".into())
                    })?;
                }
                Event::Text(text) => {
                    let value = text
                        .xml_content()
                        .map_err(|error| MaterialEffectsError::Xml(error.to_string()))?;
                    append_text(&mut tree.nodes, &stack, &value)?;
                }
                Event::CData(text) => {
                    let value = text
                        .decode()
                        .map_err(|error| MaterialEffectsError::Xml(error.to_string()))?;
                    append_text(&mut tree.nodes, &stack, &value)?;
                }
                Event::GeneralRef(reference) => {
                    let reference = std::str::from_utf8(reference.as_ref())
                        .map_err(|_| MaterialEffectsError::Xml("invalid entity".into()))?;
                    let escaped = format!("&{reference};");
                    let value = quick_xml::escape::unescape(&escaped).map_err(|_| {
                        MaterialEffectsError::Xml(format!("unsupported entity &{reference};"))
                    })?;
                    append_text(&mut tree.nodes, &stack, &value)?;
                }
                Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
                Event::DocType(_) => {
                    return Err(MaterialEffectsError::Malformed(
                        "DTDs are not accepted".into(),
                    ));
                }
                Event::Eof => break,
            }
        }
        if !stack.is_empty() {
            return Err(MaterialEffectsError::Malformed(
                "incomplete XML document".into(),
            ));
        }
        tree.root = root.ok_or_else(|| {
            MaterialEffectsError::Malformed("XML document has no root element".into())
        })?;
        Ok(tree)
    }

    fn children_of(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.nodes[index].children.iter().copied()
    }
}

#[allow(clippy::too_many_arguments)]
fn append_node(
    nodes: &mut Vec<XmlNode>,
    stack: &[usize],
    root: &mut Option<usize>,
    attribute_count: &mut usize,
    element: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    limits: &MaterialEffectsLimits,
) -> Result<usize, MaterialEffectsError> {
    if nodes.len() >= limits.max_nodes || stack.len() >= limits.max_depth {
        return Err(MaterialEffectsError::Unsafe);
    }
    let name = std::str::from_utf8(element.name().as_ref())
        .map_err(|_| MaterialEffectsError::Xml("element name is not UTF-8".into()))?
        .to_owned();
    if name.is_empty() || name.len() > MAX_REFERENCE_BYTES || !valid_xml_text(&name) {
        return Err(MaterialEffectsError::Xml("invalid element name".into()));
    }
    let mut attributes = Vec::new();
    let mut seen = HashSet::new();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| MaterialEffectsError::Xml(error.to_string()))?;
        let key = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|_| MaterialEffectsError::Xml("attribute name is not UTF-8".into()))?
            .to_owned();
        if !seen.insert(key.clone()) {
            return Err(MaterialEffectsError::Xml(format!(
                "duplicate attribute '{key}'"
            )));
        }
        let value = attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|error| MaterialEffectsError::Xml(error.to_string()))?
            .into_owned();
        if !valid_xml_text(&value) {
            return Err(MaterialEffectsError::Xml(format!(
                "invalid character in attribute '{key}'"
            )));
        }
        attributes.push(XmlAttribute { name: key, value });
    }
    *attribute_count = attribute_count.saturating_add(attributes.len());
    if *attribute_count > limits.max_attributes {
        return Err(MaterialEffectsError::Unsafe);
    }
    let index = nodes.len();
    let is_root = stack.is_empty();
    if is_root {
        if root.is_some() {
            return Err(MaterialEffectsError::Malformed(
                "multiple root elements".into(),
            ));
        }
        *root = Some(index);
    } else if let Some(&parent) = stack.last() {
        nodes[parent].children.push(index);
    }
    nodes.push(XmlNode {
        name,
        attributes: AttributeSet { attributes },
        children: Vec::new(),
        text: String::new(),
    });
    Ok(index)
}

fn append_text(
    nodes: &mut [XmlNode],
    stack: &[usize],
    text: &str,
) -> Result<(), MaterialEffectsError> {
    if !valid_xml_text(text) {
        return Err(MaterialEffectsError::Malformed(
            "invalid XML character in text".into(),
        ));
    }
    if let Some(&index) = stack.last() {
        nodes[index].text.push_str(text);
    } else if !text.trim().is_empty() {
        return Err(MaterialEffectsError::Malformed(
            "text outside the root element".into(),
        ));
    }
    Ok(())
}

fn valid_xml_text(text: &str) -> bool {
    !text.chars().any(|character| {
        character == '\0' || (character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
    })
}

fn local_name(name: &str) -> &str {
    name.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(name)
}

fn find_first_element(tree: &XmlTree, start: usize, name: &str) -> Option<usize> {
    let mut stack = vec![start];
    while let Some(index) = stack.pop() {
        for &child in tree.nodes[index].children.iter().rev() {
            if local_name(&tree.nodes[child].name).eq_ignore_ascii_case(name) {
                return Some(child);
            }
            stack.push(child);
        }
    }
    None
}

fn unmodeled(tree: &XmlTree, index: usize) -> UnmodeledElement {
    let node = &tree.nodes[index];
    UnmodeledElement {
        name: node.name.clone(),
        attributes: node.attributes.clone(),
        text: node.text.trim().to_owned(),
    }
}

fn report_unknown_attributes(
    diagnostics: &mut Vec<Diagnostic>,
    context: &str,
    attributes: &AttributeSet,
    known: &[&str],
) {
    for attribute in &attributes.attributes {
        if !known
            .iter()
            .any(|name| local_name(&attribute.name).eq_ignore_ascii_case(name))
        {
            diagnostics.push(Diagnostic {
                context: context.to_owned(),
                message: format!(
                    "unknown attribute '{}' = '{}' preserved",
                    attribute.name, attribute.value
                ),
            });
        }
    }
}

fn report_unexpected_text(diagnostics: &mut Vec<Diagnostic>, context: &str, node: &XmlNode) {
    if !node.text.trim().is_empty() {
        diagnostics.push(Diagnostic {
            context: context.to_owned(),
            message: format!("unexpected text inside <{}>", node.name),
        });
    }
}

fn validate_identifier(
    label: &str,
    value: &str,
    allow_colon: bool,
) -> Result<(), MaterialEffectsError> {
    if value.is_empty() || value.len() > MAX_NAME_BYTES {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} is empty or longer than {MAX_NAME_BYTES} bytes"
        )));
    }
    if !valid_xml_text(value) {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} contains control characters"
        )));
    }
    if value.contains('/') || value.contains('\\') || value == "." || value == ".." {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} contains a path separator or traversal component"
        )));
    }
    if value.contains("..") {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} contains a traversal component"
        )));
    }
    if !allow_colon && value.contains(':') {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} contains an unexpected ':'"
        )));
    }
    Ok(())
}

fn validate_reference(label: &str, value: &str) -> Result<(), MaterialEffectsError> {
    if value.is_empty() || value.len() > MAX_REFERENCE_BYTES {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} is empty or longer than {MAX_REFERENCE_BYTES} bytes"
        )));
    }
    if !valid_xml_text(value) {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} contains control characters"
        )));
    }
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} is absolute"
        )));
    }
    if normalized
        .as_bytes()
        .get(1)
        .is_some_and(|second| *second == b':' && normalized.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err(MaterialEffectsError::UnsafePath(format!(
            "{label} contains a drive prefix"
        )));
    }
    for component in normalized.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(MaterialEffectsError::UnsafePath(format!(
                "{label} contains an empty or traversal path component"
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Local file helpers (read-only; bounded before allocation)
// ---------------------------------------------------------------------------

pub fn read_surface_types(path: &Path) -> Result<SurfaceTypeCatalog, MaterialEffectsError> {
    let bytes = read_bounded(path, MAX_XML_BYTES)?;
    parse_surface_types(&bytes)
}

pub fn read_effect_matrix(path: &Path) -> Result<EffectMatrix, MaterialEffectsError> {
    let bytes = read_bounded(path, MAX_XML_BYTES)?;
    parse_effect_matrix(&bytes)
}

/// Read one `FXLibs/<name>.xml`; the library name is the file stem.
pub fn read_fx_library(path: &Path) -> Result<EffectLibrary, MaterialEffectsError> {
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            MaterialEffectsError::UnsafePath("FXLib path has no UTF-8 file stem".into())
        })?;
    let bytes = read_bounded(path, MAX_XML_BYTES)?;
    parse_fx_library(name, &bytes)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, MaterialEffectsError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > limit as u64 {
        return Err(MaterialEffectsError::Unsafe);
    }
    let bytes = fs::read(path)?;
    if bytes.len() > limit {
        return Err(MaterialEffectsError::Unsafe);
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Known schema names (unknown names are preserved and diagnosed)
// ---------------------------------------------------------------------------

const PHYSICS_ATTRIBUTES: &[&str] = &[
    "breakable_id",
    "break_energy",
    "hole_size",
    "hole_size_explosion",
    "hit_radius",
    "hit_points",
    "hit_points_secondary",
    "hit_maxdmg",
    "hit_lifetime",
    "pierceability",
    "damage_reduction",
    "ric_angle",
    "ric_dam_reduction",
    "ric_vel_reduction",
    "friction",
    "bouncyness",
    "iBreakability",
    "collType",
    "no_physicalize",
    "no_collide",
    "vehicle_only_collision",
    "can_shatter",
    "bullet_pierceable",
    // Observed in shipped data; kept known here so real files do not look like
    // they contain unknown schema. `hit_maxhits` is not part of
    // `SPhysicalParams` and is deliberately diagnosed as unknown.
    "elasticity",
    "footsteps",
    "softSurface",
    "breakable_2d",
];

const AI_ATTRIBUTES: &[&str] = &[
    "fImpactRadius",
    "fImpactSoundRadius",
    "fFootStepRadius",
    "proneMult",
    "crouchMult",
    "movingMult",
];

const BREAKABLE_2D_ATTRIBUTES: &[&str] = &[
    "particle_effect",
    "blast_radius",
    "blast_radius_first",
    "vert_size_spread",
    "rigid_body",
    "life_time",
    "cell_size",
    "max_patch_tris",
    "filter_angle",
    "shard_density",
    "use_edge_alpha",
    "crack_decal_scale",
    "crack_decal_mtl",
    "max_fracture",
    "full_fracture_fx",
    "fracture_fx",
    "no_procedural_full_fracture",
    "broken_mtl",
    "destroy_timeout",
    "destroy_timeout_spread",
    "break_energy",
];

const BREAKAGE_PARTICLE_ATTRIBUTES: &[&str] = &[
    "type",
    "effect",
    "particle_effect",
    "count_per_unit",
    "count_scale",
    "scale",
];

const PARTICLE_NAME_ATTRIBUTES: &[&str] = &["minscale", "maxscale", "maxscaledist"];
const DECAL_ATTRIBUTES: &[&str] = &["minscale", "maxscale", "lifetime"];

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACES: &[u8] = br#"<?xml version="1.0"?>
<SurfaceTypes>
 <SurfaceType name="mat_metal" type="metal">
  <Physics friction="0.8" can_shatter="1" hit_maxhits="4" mystery="kept"/>
  <AI fImpactRadius="5" future="x"/>
  <BreakageParticles type="joint_break" effect="breakable_objects.joint_break.metal" custom="kept"/>
  <MysteryChild foo="bar">text</MysteryChild>
 </SurfaceType>
 <SurfaceType name="mat_default">
  <BreakageParticles type="joint_break" effect="breakable_objects.joint_break.generic"/>
  <BreakageParticles type="destroy" effect="breakable_objects.Shatter.destroy"/>
 </SurfaceType>
</SurfaceTypes>"#;

    const MATRIX: &[u8] = br#"<?xml version="1.0"?>
<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">
 <Worksheet ss:Name="Sheet1">
  <Table ss:ExpandedColumnCount="5" ss:ExpandedRowCount="9">
   <Row>
    <Cell/>
    <Cell><Data ss:Type="String">mat_metal</Data></Cell>
    <Cell ss:Index="3"><Data ss:Type="String">mat_wood</Data></Cell>
    <Cell><Data ss:Type="String">mat_glass</Data></Cell>
   </Row>
   <Row>
    <Cell><Data ss:Type="String">mat_metal</Data></Cell>
    <Cell><Data ss:Type="String">collisions:rock_rock</Data></Cell>
    <Cell><Data ss:Type="String"> </Data></Cell>
   </Row>
   <Row>
    <Cell><Data ss:Type="String">PistolBullet</Data></Cell>
    <Cell ss:Index="2"><Data ss:Type="String">bulletimpacts:metal</Data></Cell>
    <Cell ss:Index="4"><Data ss:Type="String">bulletimpacts:glass</Data></Cell>
   </Row>
   <Row>
    <Cell><Data ss:Type="String">Breakage:Breakage</Data></Cell>
    <Cell><Data ss:Type="String">bad:lib:effect</Data></Cell>
   </Row>
   <Row><Cell/></Row>
   <Row ss:Index="9">
    <Cell><Data ss:Type="String">mat_wood</Data></Cell>
    <Cell><Data ss:Type="String">collisions:wood_wood</Data></Cell>
   </Row>
  </Table>
 </Worksheet>
</Workbook>"#;

    const FX_LIB: &[u8] = br#"<FXLib>
 <Effect name="hit_metal" delay="0.05" maxDist="100" future="kept">
  <Sound><Name>Sounds/Collisions:Ammo/impacts:metal</Name><Name>Sounds/Collisions:Ammo/impacts:metal2</Name></Sound>
  <RandEffect name="variants">
   <Particle><Name minscale="0.6" maxscale="1.2" maxscaledist="80">weapon_impacts.metal_impact.a</Name></Particle>
   <Decal minscale="0.03" maxscale="0.06" lifetime="10"><Filename>materials/decals/metal.tif</Filename><Material>materials/decals/metal</Material></Decal>
   <FlowGraph name="metal_hit.fg" maxdist="20"/>
   <Effect name="nested" parent="hit_metal"><Sound><Name>sounds/nested</Name></Sound></Effect>
  </RandEffect>
  <Weird attr="x"/>
 </Effect>
 <Effect name="hit_metal" parent="hit_metal"/>
</FXLib>"#;

    #[test]
    fn binary_cryxmlb_sources_match_authored_xml() {
        let compiled = crate::cryxml::encode_xml(SURFACES, MAX_XML_BYTES).unwrap();
        assert_eq!(
            parse_surface_types(&compiled).unwrap(),
            parse_surface_types(SURFACES).unwrap()
        );
        let compiled = crate::cryxml::encode_xml(FX_LIB, MAX_XML_BYTES).unwrap();
        assert_eq!(
            parse_fx_library("test", &compiled).unwrap(),
            parse_fx_library("test", FX_LIB).unwrap()
        );
    }

    #[test]
    fn surfaces_keep_unknown_attributes_and_distinguish_breakage_fallback() {
        let catalog = parse_surface_types(SURFACES).unwrap();
        assert_eq!(catalog.surfaces.len(), 2);
        let metal = catalog.get("MAT_METAL").unwrap();
        assert_eq!(metal.name, "mat_metal");
        assert_eq!(metal.type_name.as_deref(), Some("metal"));
        assert!(metal.flags.can_shatter);
        assert!(!metal.flags.no_collide);
        assert_eq!(metal.physics_attribute("friction"), Some("0.8"));
        assert_eq!(metal.physics_attribute("mystery"), Some("kept"));
        assert_eq!(metal.physics_attribute("hit_maxhits"), Some("4"));
        assert_eq!(metal.ai_attribute("future"), Some("x"));
        assert_eq!(metal.unknown_elements.len(), 1);
        assert!(
            catalog
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("mystery"))
        );
        assert!(
            catalog
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("hit_maxhits"))
        );
        assert!(
            catalog
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("custom"))
        );
        assert!(
            catalog
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("MysteryChild"))
        );

        assert!(catalog.get("mat_metal_extra").is_none());
        assert!(catalog.get("mat_metal").is_some());
        match catalog.breakage_particles("mat_metal", "joint_break", false) {
            BreakageLookup::Specific(particle) => {
                assert_eq!(
                    particle.effect.as_deref(),
                    Some("breakable_objects.joint_break.metal")
                );
            }
            other => panic!("expected specific breakage entry, got {other:?}"),
        }
        match catalog.breakage_particles("mat_metal", "destroy", true) {
            BreakageLookup::Default(particle) => {
                assert_eq!(
                    particle.effect.as_deref(),
                    Some("breakable_objects.Shatter.destroy")
                );
            }
            other => panic!("expected default breakage fallback, got {other:?}"),
        }
        assert!(matches!(
            catalog.breakage_particles("mat_metal", "destroy", false),
            BreakageLookup::Missing(_)
        ));
        assert!(matches!(
            catalog.breakage_particles("MAT_METAL_EXTRA", "destroy", true),
            BreakageLookup::NotDefined
        ));
        assert_eq!(
            catalog.surfaces[0].breakage_particles[0]
                .effect_attribute
                .as_deref(),
            Some("effect")
        );
        assert_eq!(
            catalog.surfaces[0].breakage_particles[0]
                .attributes
                .get("custom"),
            Some("kept")
        );
    }

    #[test]
    fn workbook_sparse_indices_blanks_and_custom_rows_map_exactly() {
        let original = MATRIX.to_vec();
        let matrix = parse_effect_matrix(&original).unwrap();
        assert_eq!(matrix.declared_columns, Some(5));
        assert_eq!(matrix.declared_rows, Some(9));
        assert_eq!(
            matrix
                .columns
                .iter()
                .map(|column| (column.index, column.name.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, ""), (1, "mat_metal"), (2, "mat_wood"), (3, "mat_glass")]
        );
        assert_eq!(matrix.rows.len(), 5);
        assert_eq!(matrix.rows[0].index, 1);
        assert_eq!(matrix.rows[0].kind, MatrixRowKind::Surface);
        assert_eq!(matrix.rows[1].kind, MatrixRowKind::Custom);
        assert_eq!(matrix.rows[3].kind, MatrixRowKind::Blank);
        assert_eq!(matrix.rows[4].index, 8);
        assert_eq!(matrix.rows[4].kind, MatrixRowKind::Surface);

        match matrix.cell("mat_metal", "mat_metal") {
            MatrixLookup::Effect { reference, .. } => {
                assert_eq!(reference.library, "collisions");
                assert_eq!(reference.effect, "rock_rock");
                assert_eq!(reference.raw, "collisions:rock_rock");
            }
            other => panic!("expected effect, got {other:?}"),
        }
        assert!(matches!(
            matrix.cell("mat_metal", "mat_wood"),
            MatrixLookup::Blank { .. }
        ));
        assert!(matches!(
            matrix.cell("mat_metal", "mat_glass"),
            MatrixLookup::Missing { .. }
        ));
        match matrix.cell("PistolBullet", "mat_glass") {
            MatrixLookup::Effect { reference, .. } => {
                assert_eq!(reference.library, "bulletimpacts");
                assert_eq!(reference.effect, "glass");
            }
            other => panic!("expected custom-row effect, got {other:?}"),
        }
        assert!(matches!(
            matrix.cell("PistolBullet", "mat_wood"),
            MatrixLookup::Missing { .. }
        ));
        match matrix.cell("Breakage:Breakage", "mat_metal") {
            MatrixLookup::Invalid { cell, .. } => {
                assert_eq!(cell.raw, "bad:lib:effect");
                assert!(cell.reason.contains("lib:effect"));
            }
            other => panic!("expected invalid cell, got {other:?}"),
        }
        match matrix.cell("unknown", "mat_metal") {
            MatrixLookup::NotDefined {
                row_found,
                column_found,
                ..
            } => {
                assert!(!row_found);
                assert!(column_found);
            }
            other => panic!("expected not-defined row, got {other:?}"),
        }
        match matrix.cell("mat_metal", "MAT_METAL_EXTRA") {
            MatrixLookup::NotDefined {
                row_found,
                column_found,
                ..
            } => {
                assert!(row_found);
                assert!(!column_found);
            }
            other => panic!("expected not-defined column, got {other:?}"),
        }
        match matrix.cell("MAT_METAL", "Mat_Metal") {
            MatrixLookup::Effect { reference, .. } => assert_eq!(reference.effect, "rock_rock"),
            other => panic!("expected case-insensitive exact effect, got {other:?}"),
        }
        assert!(
            matrix
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("invalid effect cell"))
        );
        assert_eq!(original, MATRIX, "parsing must not mutate input");
    }

    #[test]
    fn workbook_cell_spans_and_merges_advance_columns() {
        let xml = br#"<Workbook xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">
 <Worksheet><Table>
  <Row>
   <Cell/>
   <Cell ss:Index="2" ss:Span="1"><Data ss:Type="String">mat_metal</Data></Cell>
   <Cell><Data ss:Type="String">mat_wood</Data></Cell>
  </Row>
  <Row>
   <Cell><Data ss:Type="String">PistolBullet</Data></Cell>
   <Cell ss:MergeAcross="1"><Data ss:Type="String">collisions:both</Data></Cell>
   <Cell><Data ss:Type="String">collisions:wood</Data></Cell>
  </Row>
 </Table></Worksheet>
</Workbook>"#;
        let matrix = parse_effect_matrix(xml).unwrap();
        assert_eq!(
            matrix
                .columns
                .iter()
                .map(|column| (column.index, column.name.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, ""), (1, "mat_metal"), (3, "mat_wood")]
        );
        match matrix.cell("PistolBullet", "mat_metal") {
            MatrixLookup::Effect { reference, .. } => assert_eq!(reference.effect, "both"),
            other => panic!("expected spanned effect, got {other:?}"),
        }
        match matrix.cell("PistolBullet", "mat_wood") {
            MatrixLookup::Effect { reference, .. } => assert_eq!(reference.effect, "wood"),
            other => panic!("expected post-merge effect, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_rows_and_columns_are_reported_and_first_wins() {
        let xml = br#"<Workbook><Worksheet><Table>
 <Row><Cell/><Cell><Data>mat_metal</Data></Cell><Cell><Data>MAT_METAL</Data></Cell></Row>
 <Row><Cell><Data>mat_metal</Data></Cell><Cell><Data>collisions:first</Data></Cell><Cell><Data>collisions:second</Data></Cell></Row>
 <Row><Cell><Data>mat_metal</Data></Cell><Cell><Data>collisions:third</Data></Cell></Row>
 </Table></Worksheet></Workbook>"#;
        let matrix = parse_effect_matrix(xml).unwrap();
        assert_eq!(matrix.columns.len(), 3);
        assert_eq!(matrix.rows.len(), 2);
        assert!(
            matrix
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate column"))
        );
        assert!(
            matrix
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate row"))
        );
        match matrix.cell("mat_metal", "mat_metal") {
            MatrixLookup::Effect { reference, .. } => assert_eq!(reference.effect, "first"),
            other => panic!("expected first duplicate row effect, got {other:?}"),
        }
    }

    #[test]
    fn fx_library_preserves_groups_parents_and_resource_references() {
        let original = FX_LIB.to_vec();
        let library = parse_fx_library("bulletimpacts", &original).unwrap();
        assert_eq!(library.name, "bulletimpacts");
        assert_eq!(library.effects.len(), 2);
        let effect = library.effect("HIT_METAL").unwrap();
        assert_eq!(effect.parent, None);
        assert_eq!(effect.delay.as_deref(), Some("0.05"));
        assert_eq!(effect.max_distance.as_deref(), Some("100"));
        assert_eq!(effect.attributes.get("future"), Some("kept"));
        assert_eq!(
            effect
                .resources
                .iter()
                .map(|resource| (
                    resource.kind,
                    resource.reference.as_str(),
                    resource.group.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    EffectResourceKind::Sound,
                    "Sounds/Collisions:Ammo/impacts:metal",
                    None
                ),
                (
                    EffectResourceKind::Sound,
                    "Sounds/Collisions:Ammo/impacts:metal2",
                    None
                ),
                (
                    EffectResourceKind::Particle,
                    "weapon_impacts.metal_impact.a",
                    Some("variants")
                ),
                (
                    EffectResourceKind::DecalTexture,
                    "materials/decals/metal.tif",
                    Some("variants")
                ),
                (
                    EffectResourceKind::DecalMaterial,
                    "materials/decals/metal",
                    Some("variants")
                ),
                (
                    EffectResourceKind::FlowGraph,
                    "metal_hit.fg",
                    Some("variants")
                ),
            ]
        );
        assert_eq!(effect.nested_effects.len(), 1);
        assert_eq!(effect.nested_effects[0].name, "nested");
        assert_eq!(
            effect.nested_effects[0].parent.as_deref(),
            Some("hit_metal")
        );
        assert_eq!(effect.resources[2].attributes.get("minscale"), Some("0.6"));
        assert_eq!(effect.resources[5].attributes.get("maxdist"), Some("20"));
        assert_eq!(effect.unknown_elements.len(), 1);
        assert_eq!(library.effects[1].parent.as_deref(), Some("hit_metal"));
        assert!(
            library
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate effect"))
        );
        assert!(
            library
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("future"))
        );
        assert!(
            library
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("Weird"))
        );
        assert_eq!(original, FX_LIB, "parsing must not mutate input");
    }

    #[test]
    fn catalog_resolves_library_and_effect_without_inventing_defaults() {
        let surfaces = parse_surface_types(SURFACES).unwrap();
        let matrix = parse_effect_matrix(MATRIX).unwrap();
        let library = parse_fx_library("bulletimpacts", FX_LIB).unwrap();
        let catalog = MaterialEffectsCatalog::new(surfaces, matrix, vec![library]);
        let reference = EffectReference {
            raw: "bulletimpacts:metal".into(),
            library: "bulletimpacts".into(),
            effect: "metal".into(),
        };
        assert!(matches!(
            catalog.effect(&reference),
            EffectLookup::EffectNotDefined { .. }
        ));
        let hit = EffectReference {
            raw: "bulletimpacts:hit_metal".into(),
            library: "BULLETIMPACTS".into(),
            effect: "HIT_METAL".into(),
        };
        match catalog.effect(&hit) {
            EffectLookup::Found(effect) => assert_eq!(effect.name, "hit_metal"),
            other => panic!("expected effect, got {other:?}"),
        }
        assert!(matches!(
            catalog.effect_by_name("missing", "hit_metal"),
            EffectLookup::LibraryNotDefined("missing")
        ));
    }

    #[test]
    fn malformed_and_unsafe_documents_fail_closed() {
        assert!(matches!(
            parse_surface_types(b"<SurfaceTypes><SurfaceType/></SurfaceTypes>"),
            Err(MaterialEffectsError::Malformed(_))
        ));
        assert!(matches!(
            parse_surface_types(br#"<SurfaceTypes><SurfaceType name="../evil"/></SurfaceTypes>"#),
            Err(MaterialEffectsError::UnsafePath(_))
        ));
        assert!(matches!(
            parse_effect_matrix(b"<NotAWorkbook/>"),
            Err(MaterialEffectsError::Malformed(_))
        ));
        assert!(matches!(
            parse_effect_matrix(b"<Workbook><Worksheet/></Workbook>"),
            Err(MaterialEffectsError::Malformed(_))
        ));
        assert!(matches!(
            parse_effect_matrix(
                br#"<Workbook><Worksheet><Table><Row><Cell/></Row><Row><Cell ss:Index="1"/><Cell ss:Index="1"/></Row></Table></Worksheet></Workbook>"#
            ),
            Err(MaterialEffectsError::Malformed(_))
        ));
        assert!(matches!(
            parse_fx_library("ok", b"<!DOCTYPE FXLib><FXLib/>"),
            Err(MaterialEffectsError::Malformed(_))
        ));
        assert!(matches!(
            parse_fx_library("../evil", FX_LIB),
            Err(MaterialEffectsError::UnsafePath(_))
        ));
        assert!(matches!(
            parse_fx_library("ok", br#"<FXLib><Effect name="bad/name"/></FXLib>"#),
            Err(MaterialEffectsError::UnsafePath(_))
        ));
        assert!(matches!(
            parse_fx_library(
                "ok",
                br#"<FXLib><Effect name="x"><Sound><Name>../../evil</Name></Sound></Effect></FXLib>"#
            ),
            Err(MaterialEffectsError::UnsafePath(_))
        ));
        assert!(matches!(
            parse_surface_types(b"<SurfaceTypes><SurfaceType name=\"x\">"),
            Err(MaterialEffectsError::Xml(_)) | Err(MaterialEffectsError::Malformed(_))
        ));
    }

    #[test]
    fn explicit_limits_reject_oversized_inputs() {
        let tiny = MaterialEffectsLimits {
            max_bytes: 32,
            ..MaterialEffectsLimits::default()
        };
        assert!(matches!(
            parse_surface_types_with_limits(SURFACES, &tiny),
            Err(MaterialEffectsError::Unsafe)
        ));

        let narrow = MaterialEffectsLimits {
            max_matrix_columns: 2,
            ..MaterialEffectsLimits::default()
        };
        assert!(matches!(
            parse_effect_matrix_with_limits(MATRIX, &narrow),
            Err(MaterialEffectsError::Unsafe)
        ));

        let shallow = MaterialEffectsLimits {
            max_depth: 1,
            ..MaterialEffectsLimits::default()
        };
        assert!(matches!(
            parse_surface_types_with_limits(SURFACES, &shallow),
            Err(MaterialEffectsError::Unsafe)
        ));

        let one_effect = MaterialEffectsLimits {
            max_effects: 1,
            ..MaterialEffectsLimits::default()
        };
        let two_effects = br#"<FXLib><Effect name="a"/><Effect name="b"/></FXLib>"#;
        assert!(matches!(
            parse_fx_library_with_limits("ok", two_effects, &one_effect),
            Err(MaterialEffectsError::Unsafe)
        ));
    }

    #[test]
    fn matrix_effect_cells_reject_traversal_without_failing_the_sheet() {
        let xml = br#"<Workbook><Worksheet><Table>
 <Row><Cell/><Cell><Data>mat_metal</Data></Cell></Row>
 <Row><Cell><Data>PistolBullet</Data></Cell><Cell><Data>../evil:../effect</Data></Cell></Row>
 </Table></Worksheet></Workbook>"#;
        let matrix = parse_effect_matrix(xml).unwrap();
        match matrix.cell("PistolBullet", "mat_metal") {
            MatrixLookup::Invalid { cell, .. } => assert!(cell.reason.contains("Unsafe")),
            other => panic!("expected invalid traversal cell, got {other:?}"),
        }
    }

    #[test]
    fn surface_coverage_reports_missing_rows_and_columns() {
        let surfaces = parse_surface_types(SURFACES).unwrap();
        let matrix = parse_effect_matrix(MATRIX).unwrap();
        let coverage = matrix.surface_coverage(&surfaces);
        assert_eq!(coverage.len(), 2);
        assert_eq!(
            coverage
                .iter()
                .find(|item| item.surface == "mat_metal")
                .unwrap(),
            &SurfaceMatrixCoverage {
                surface: "mat_metal".into(),
                has_row: true,
                has_column: true,
            }
        );
        assert_eq!(
            matrix.surfaces_without_row_or_column(&surfaces),
            vec!["mat_default".to_owned()]
        );
    }

    #[test]
    #[ignore = "requires EVOLVE_MATERIAL_EFFECTS_DIR pointing at an extracted Libs/MaterialEffects directory"]
    fn real_sdk_material_effects_inventory() {
        let Ok(directory) = std::env::var("EVOLVE_MATERIAL_EFFECTS_DIR") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let surfaces = read_surface_types(&directory.join("SurfaceTypes.xml")).unwrap();
        let matrix = read_effect_matrix(&directory.join("MaterialEffects.xml")).unwrap();
        let mut libraries = Vec::new();
        for entry in std::fs::read_dir(directory.join("FXLibs")).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("xml"))
            {
                libraries.push(read_fx_library(&path).unwrap());
            }
        }
        let effect_cells = matrix
            .rows
            .iter()
            .flat_map(|row| row.cells.iter())
            .filter(|entry| matches!(entry.value, MatrixCellValue::Effect(_)))
            .count();
        eprintln!(
            "surfaces={} columns={} rows={} effect_cells={} libraries={} effects={}",
            surfaces.surfaces.len(),
            matrix
                .columns
                .iter()
                .filter(|column| !column.name.is_empty())
                .count(),
            matrix.rows.len(),
            effect_cells,
            libraries.len(),
            libraries
                .iter()
                .map(|library| library.effects.len())
                .sum::<usize>(),
        );
        assert!(!surfaces.surfaces.is_empty());
        assert!(!matrix.rows.is_empty());
        assert!(effect_cells > 0);
        assert!(!libraries.is_empty());
    }
}
