//! Cross-format source locations and parent-relative containment chains.

use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

#[cfg(feature = "schemas")]
use schemars::{JsonSchema, schema::Schema};

/// Whether an ordinal is counted from zero or one.
///
/// Every format component carrying a page, slide, cell, frame, token, output,
/// member, track, or record ordinal uses this type rather than relying on an
/// undocumented convention.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexBase {
    Zero,
    One,
}

impl IndexBase {
    const fn minimum(self) -> u64 {
        match self {
            Self::Zero => 0,
            Self::One => 1,
        }
    }
}

/// One explicitly based ordinal.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexPosition {
    pub value: u64,
    pub base: IndexBase,
}

impl IndexPosition {
    pub fn new(value: u64, base: IndexBase) -> Result<Self, SourceLocatorError> {
        let position = Self { value, base };
        position.validate("index")?;
        Ok(position)
    }

    /// Construct a public, human-facing one-based ordinal.
    pub fn one_based(value: u64) -> Result<Self, SourceLocatorError> {
        Self::new(value, IndexBase::One)
    }

    /// Construct a machine-facing zero-based ordinal.
    pub const fn zero_based(value: u64) -> Self {
        Self {
            value,
            base: IndexBase::Zero,
        }
    }

    fn validate(self, field: &str) -> Result<(), SourceLocatorError> {
        if self.value < self.base.minimum() {
            return Err(SourceLocatorError::InvalidIndex {
                field: field.to_string(),
                value: self.value,
                base: self.base,
            });
        }
        Ok(())
    }
}

/// A half-open range of ordinals using one declared base.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexRange {
    pub start: u64,
    pub end: u64,
    pub base: IndexBase,
}

impl IndexRange {
    pub fn new(start: u64, end: u64, base: IndexBase) -> Result<Self, SourceLocatorError> {
        let range = Self { start, end, base };
        range.validate("index_range")?;
        Ok(range)
    }

    fn validate(self, field: &str) -> Result<(), SourceLocatorError> {
        if self.start < self.base.minimum() {
            return Err(SourceLocatorError::InvalidIndex {
                field: format!("{field}.start"),
                value: self.start,
                base: self.base,
            });
        }
        if self.end < self.start {
            return Err(SourceLocatorError::ReversedRange {
                field: field.to_string(),
                start: self.start,
                end: self.end,
            });
        }
        Ok(())
    }
}

/// A sheet cell address with explicit row and column bases.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CellAddress {
    pub row: u64,
    pub column: u64,
    pub base: IndexBase,
}

impl CellAddress {
    pub fn new(row: u64, column: u64, base: IndexBase) -> Result<Self, SourceLocatorError> {
        let address = Self { row, column, base };
        address.validate("cell")?;
        Ok(address)
    }

    pub fn a1(row: u64, column: u64) -> Result<Self, SourceLocatorError> {
        Self::new(row, column, IndexBase::One)
    }

    fn validate(self, field: &str) -> Result<(), SourceLocatorError> {
        IndexPosition {
            value: self.row,
            base: self.base,
        }
        .validate(&format!("{field}.row"))?;
        IndexPosition {
            value: self.column,
            base: self.base,
        }
        .validate(&format!("{field}.column"))
    }
}

/// Units used by rectangular source regions.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateUnit {
    Points,
    Pixels,
    Normalized,
}

/// Coordinate-system origin for rectangular source regions.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateOrigin {
    TopLeft,
    BottomLeft,
}

/// A rectangle in a declared coordinate system.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub unit: CoordinateUnit,
    pub origin: CoordinateOrigin,
}

impl BoundingBox {
    pub fn validate(self, field: &str) -> Result<(), SourceLocatorError> {
        let values = [self.x, self.y, self.width, self.height];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(SourceLocatorError::InvalidBoundingBox {
                field: field.to_string(),
                reason: "coordinates must be finite".into(),
            });
        }
        if self.width < 0.0 || self.height < 0.0 {
            return Err(SourceLocatorError::InvalidBoundingBox {
                field: field.to_string(),
                reason: "width and height cannot be negative".into(),
            });
        }
        if self.unit == CoordinateUnit::Normalized
            && (self.x < 0.0
                || self.y < 0.0
                || self.width > 1.0
                || self.height > 1.0
                || self.x + self.width > 1.0
                || self.y + self.height > 1.0)
        {
            return Err(SourceLocatorError::InvalidBoundingBox {
                field: field.to_string(),
                reason: "normalized rectangles must fit within [0, 1]".into(),
            });
        }
        Ok(())
    }
}

/// A confidence in the inclusive range `[0, 1]`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(transparent)]
pub struct LocatorConfidence(f64);

impl LocatorConfidence {
    pub fn new(value: f64) -> Result<Self, SourceLocatorError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(SourceLocatorError::InvalidConfidence(value));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for LocatorConfidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schemas")]
impl JsonSchema for LocatorConfidence {
    fn schema_name() -> String {
        "LocatorConfidence".into()
    }

    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> Schema {
        let mut schema = f64::json_schema(generator);
        if let Schema::Object(object) = &mut schema {
            let number = object.number.get_or_insert_with(Default::default);
            number.minimum = Some(0.0);
            number.maximum = Some(1.0);
        }
        schema
    }
}

// UTF-8 text compatibility range. Byte and human positions are half-open.
// Byte offsets are zero-based. Lines and Unicode-scalar columns are one-based.
// Keep this as a non-rustdoc comment: SourceRange is embedded in legacy public
// schemas whose generated bytes must not change during the locator migration.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub byte_start: usize,
    pub byte_end: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl SourceRange {
    pub fn new(byte_start: usize, byte_end: usize, index: &LineIndex) -> Self {
        let start = index.line_column(byte_start);
        let end = index.line_column(byte_end);
        Self {
            byte_start,
            byte_end,
            start_line: start.line,
            start_column: start.column,
            end_line: end.line,
            end_column: end.column,
        }
    }

    pub fn validate(&self) -> Result<(), SourceLocatorError> {
        if self.byte_end < self.byte_start {
            return Err(SourceLocatorError::ReversedRange {
                field: "text.byte".into(),
                start: self.byte_start as u64,
                end: self.byte_end as u64,
            });
        }
        if self.start_line == 0
            || self.start_column == 0
            || self.end_line == 0
            || self.end_column == 0
        {
            return Err(SourceLocatorError::InvalidTextPosition(
                "lines and columns are one-based".into(),
            ));
        }
        if (self.end_line, self.end_column) < (self.start_line, self.start_column) {
            return Err(SourceLocatorError::InvalidTextPosition(
                "end position precedes start position".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    char_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (idx, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        let char_starts = text.char_indices().map(|(idx, _)| idx).collect();
        Self {
            line_starts,
            char_starts,
        }
    }

    pub fn line_column(&self, byte_offset: usize) -> LineColumn {
        let line_idx = match self.line_starts.binary_search(&byte_offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx];
        let line_char_start = self
            .char_starts
            .partition_point(|start| *start < line_start);
        let offset_char_start = self
            .char_starts
            .partition_point(|start| *start < byte_offset);
        LineColumn {
            line: line_idx + 1,
            column: offset_char_start.saturating_sub(line_char_start) + 1,
        }
    }
}

/// A reference from derived content back to the nodes and declared transforms
/// that produced it.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivedNodeReference {
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub source_node_ids: Vec<String>,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub derivation_steps: Vec<String>,
}

impl DerivedNodeReference {
    pub fn new(
        source_node_ids: Vec<String>,
        derivation_steps: Vec<String>,
    ) -> Result<Self, SourceLocatorError> {
        let reference = Self {
            source_node_ids,
            derivation_steps,
        };
        reference.validate()?;
        Ok(reference)
    }

    fn validate(&self) -> Result<(), SourceLocatorError> {
        if self.source_node_ids.is_empty() || self.source_node_ids.iter().any(|id| id.is_empty()) {
            return Err(SourceLocatorError::InvalidDerivation(
                "at least one non-empty source node ID is required".into(),
            ));
        }
        if self.derivation_steps.is_empty()
            || self.derivation_steps.iter().any(|step| step.is_empty())
        {
            return Err(SourceLocatorError::InvalidDerivation(
                "at least one non-empty derivation step is required".into(),
            ));
        }
        Ok(())
    }
}

/// Precision and derivation metadata for a complete locator chain.
///
/// The tagged shape makes invalid states unrepresentable: approximate
/// locations always carry confidence, and synthetic locations always carry
/// source-node and derivation-step references.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "precision", rename_all = "snake_case")]
pub enum LocatorPrecision {
    Exact {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        derived_from: Option<DerivedNodeReference>,
    },
    Approximate {
        confidence: LocatorConfidence,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        derived_from: Option<DerivedNodeReference>,
    },
    Synthetic {
        derived_from: DerivedNodeReference,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<LocatorConfidence>,
    },
}

impl LocatorPrecision {
    fn validate(&self) -> Result<(), SourceLocatorError> {
        match self {
            Self::Exact { derived_from } | Self::Approximate { derived_from, .. } => {
                if let Some(reference) = derived_from {
                    reference.validate()?;
                }
            }
            Self::Synthetic {
                derived_from,
                confidence: _,
            } => derived_from.validate()?,
        }
        Ok(())
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::Exact { .. } => "exact",
            Self::Approximate { .. } => "approximate",
            Self::Synthetic { .. } => "synthetic",
        }
    }

    pub const fn confidence(&self) -> Option<LocatorConfidence> {
        match self {
            Self::Exact { .. } => None,
            Self::Approximate { confidence, .. } => Some(*confidence),
            Self::Synthetic { confidence, .. } => *confidence,
        }
    }

    pub fn derived_from(&self) -> Option<&DerivedNodeReference> {
        match self {
            Self::Exact { derived_from } | Self::Approximate { derived_from, .. } => {
                derived_from.as_ref()
            }
            Self::Synthetic { derived_from, .. } => Some(derived_from),
        }
    }
}

/// One location inside its immediately preceding parent component.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocationComponent {
    TextRange {
        byte_start: usize,
        byte_end: usize,
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    },
    PdfRegion {
        page: IndexPosition,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bbox: Option<BoundingBox>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation_degrees: Option<i16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<IndexRange>,
    },
    OoxmlPart {
        part: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        paragraph: Option<IndexPosition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<IndexPosition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        table: Option<IndexPosition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row: Option<IndexPosition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column: Option<IndexPosition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object_id: Option<String>,
    },
    SlideRegion {
        slide: IndexPosition,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shape_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bbox: Option<BoundingBox>,
    },
    SheetRange {
        sheet: String,
        start_cell: CellAddress,
        end_cell: CellAddress,
    },
    NotebookCell {
        index: IndexPosition,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_index: Option<IndexPosition>,
    },
    EmailPart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mime_path: Vec<IndexPosition>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header: Option<String>,
    },
    ArchiveMember {
        member_path: String,
        member_index: IndexPosition,
    },
    ImageRegion {
        frame: IndexPosition,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bbox: Option<BoundingBox>,
    },
    MediaTime {
        start_ms: u64,
        end_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<IndexPosition>,
    },
    RecordRange {
        collection: String,
        records: IndexRange,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field: Option<String>,
    },
    JsonPointer {
        pointer: String,
    },
    XmlPath {
        path: String,
    },
}

impl From<SourceRange> for LocationComponent {
    fn from(range: SourceRange) -> Self {
        Self::TextRange {
            byte_start: range.byte_start,
            byte_end: range.byte_end,
            start_line: range.start_line,
            start_column: range.start_column,
            end_line: range.end_line,
            end_column: range.end_column,
        }
    }
}

impl LocationComponent {
    pub fn as_text_range(&self) -> Option<SourceRange> {
        match self {
            Self::TextRange {
                byte_start,
                byte_end,
                start_line,
                start_column,
                end_line,
                end_column,
            } => Some(SourceRange {
                byte_start: *byte_start,
                byte_end: *byte_end,
                start_line: *start_line,
                start_column: *start_column,
                end_line: *end_line,
                end_column: *end_column,
            }),
            _ => None,
        }
    }

    fn validate(&self, index: usize) -> Result<(), SourceLocatorError> {
        let field = |suffix: &str| format!("components[{index}].{suffix}");
        match self {
            Self::TextRange { .. } => {
                self.as_text_range()
                    .expect("text range variant")
                    .validate()?;
            }
            Self::PdfRegion {
                page,
                bbox,
                rotation_degrees,
                tokens,
            } => {
                page.validate(&field("page"))?;
                if let Some(bbox) = bbox {
                    bbox.validate(&field("bbox"))?;
                }
                if let Some(rotation) = rotation_degrees
                    && !matches!(rotation.rem_euclid(360), 0 | 90 | 180 | 270)
                {
                    return Err(SourceLocatorError::InvalidComponent {
                        component: index,
                        reason: "PDF rotation must be a multiple of 90 degrees".into(),
                    });
                }
                if let Some(tokens) = tokens {
                    tokens.validate(&field("tokens"))?;
                }
            }
            Self::OoxmlPart {
                part,
                paragraph,
                run,
                table,
                row,
                column,
                object_id,
            } => {
                require_non_empty(part, index, "OOXML part")?;
                for (name, position) in [
                    ("paragraph", paragraph),
                    ("run", run),
                    ("table", table),
                    ("row", row),
                    ("column", column),
                ] {
                    if let Some(position) = position {
                        position.validate(&field(name))?;
                    }
                }
                validate_optional_string(object_id, index, "object_id")?;
            }
            Self::SlideRegion {
                slide,
                shape_id,
                bbox,
            } => {
                slide.validate(&field("slide"))?;
                validate_optional_string(shape_id, index, "shape_id")?;
                if let Some(bbox) = bbox {
                    bbox.validate(&field("bbox"))?;
                }
            }
            Self::SheetRange {
                sheet,
                start_cell,
                end_cell,
            } => {
                require_non_empty(sheet, index, "sheet")?;
                start_cell.validate(&field("start_cell"))?;
                end_cell.validate(&field("end_cell"))?;
                if start_cell.base != end_cell.base {
                    return Err(SourceLocatorError::MixedIndexBase(field("cells")));
                }
                if end_cell.row < start_cell.row || end_cell.column < start_cell.column {
                    return Err(SourceLocatorError::InvalidComponent {
                        component: index,
                        reason: "sheet end cell must not precede the start cell".into(),
                    });
                }
            }
            Self::NotebookCell {
                index: cell,
                cell_id,
                output_index,
            } => {
                cell.validate(&field("index"))?;
                validate_optional_string(cell_id, index, "cell_id")?;
                if let Some(output) = output_index {
                    output.validate(&field("output_index"))?;
                }
            }
            Self::EmailPart {
                message_id,
                mime_path,
                header,
            } => {
                validate_optional_string(message_id, index, "message_id")?;
                validate_optional_string(header, index, "header")?;
                if message_id.is_none() && mime_path.is_empty() && header.is_none() {
                    return Err(SourceLocatorError::InvalidComponent {
                        component: index,
                        reason: "email part needs a message ID, MIME path, or header".into(),
                    });
                }
                for (part, position) in mime_path.iter().enumerate() {
                    position.validate(&field(&format!("mime_path[{part}]")))?;
                }
            }
            Self::ArchiveMember {
                member_path,
                member_index,
            } => {
                require_non_empty(member_path, index, "archive member path")?;
                member_index.validate(&field("member_index"))?;
            }
            Self::ImageRegion { frame, bbox } => {
                frame.validate(&field("frame"))?;
                if let Some(bbox) = bbox {
                    bbox.validate(&field("bbox"))?;
                }
            }
            Self::MediaTime {
                start_ms,
                end_ms,
                track,
            } => {
                if end_ms < start_ms {
                    return Err(SourceLocatorError::ReversedRange {
                        field: field("time_ms"),
                        start: *start_ms,
                        end: *end_ms,
                    });
                }
                if let Some(track) = track {
                    track.validate(&field("track"))?;
                }
            }
            Self::RecordRange {
                collection,
                records,
                field: record_field,
            } => {
                require_non_empty(collection, index, "record collection")?;
                records.validate(&field("records"))?;
                validate_optional_string(record_field, index, "field")?;
            }
            Self::JsonPointer { pointer } => {
                if !valid_json_pointer(pointer) {
                    return Err(SourceLocatorError::InvalidComponent {
                        component: index,
                        reason: "JSON Pointer must use RFC 6901 string syntax".into(),
                    });
                }
            }
            Self::XmlPath { path } => {
                if path.is_empty() || !path.starts_with('/') {
                    return Err(SourceLocatorError::InvalidComponent {
                        component: index,
                        reason: "XML path must be a non-empty absolute path".into(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// An outermost-to-innermost containment chain.
///
/// Component `n` is interpreted relative to component `n - 1`. For example,
/// `[archive_member, email_part, ooxml_part, text_range]` identifies text in an
/// OOXML part attached to an email stored in an archive without flattening any
/// parent coordinate system.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SourceLocator {
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    components: Vec<LocationComponent>,
    #[serde(flatten)]
    precision: LocatorPrecision,
}

#[derive(Deserialize)]
struct SourceLocatorWire {
    components: Vec<LocationComponent>,
    #[serde(flatten)]
    precision: LocatorPrecision,
}

impl<'de> Deserialize<'de> for SourceLocator {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SourceLocatorWire::deserialize(deserializer)?;
        Self::new(wire.components, wire.precision).map_err(serde::de::Error::custom)
    }
}

impl SourceLocator {
    pub fn new(
        components: Vec<LocationComponent>,
        precision: LocatorPrecision,
    ) -> Result<Self, SourceLocatorError> {
        let locator = Self {
            components,
            precision,
        };
        locator.validate()?;
        Ok(locator)
    }

    pub fn exact(component: impl Into<LocationComponent>) -> Result<Self, SourceLocatorError> {
        Self::new(
            vec![component.into()],
            LocatorPrecision::Exact { derived_from: None },
        )
    }

    pub fn approximate(
        component: impl Into<LocationComponent>,
        confidence: LocatorConfidence,
    ) -> Result<Self, SourceLocatorError> {
        Self::new(
            vec![component.into()],
            LocatorPrecision::Approximate {
                confidence,
                derived_from: None,
            },
        )
    }

    pub fn synthetic(
        component: impl Into<LocationComponent>,
        derived_from: DerivedNodeReference,
    ) -> Result<Self, SourceLocatorError> {
        Self::new(
            vec![component.into()],
            LocatorPrecision::Synthetic {
                derived_from,
                confidence: None,
            },
        )
    }

    /// Append a child component interpreted relative to the current innermost
    /// component.
    pub fn nested(
        mut self,
        component: impl Into<LocationComponent>,
    ) -> Result<Self, SourceLocatorError> {
        self.components.push(component.into());
        self.validate()?;
        Ok(self)
    }

    pub fn components(&self) -> &[LocationComponent] {
        &self.components
    }

    pub fn precision(&self) -> &LocatorPrecision {
        &self.precision
    }

    pub fn innermost(&self) -> &LocationComponent {
        self.components
            .last()
            .expect("validated locators always contain a component")
    }

    pub fn validate(&self) -> Result<(), SourceLocatorError> {
        if self.components.is_empty() {
            return Err(SourceLocatorError::EmptyComponents);
        }
        self.precision.validate()?;
        for (index, component) in self.components.iter().enumerate() {
            component.validate(index)?;
        }
        Ok(())
    }
}

impl TryFrom<SourceRange> for SourceLocator {
    type Error = SourceLocatorError;

    fn try_from(range: SourceRange) -> Result<Self, Self::Error> {
        Self::exact(range)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SourceLocatorError {
    #[error("a source locator must contain at least one component")]
    EmptyComponents,
    #[error("{field} uses invalid {base:?}-based index {value}")]
    InvalidIndex {
        field: String,
        value: u64,
        base: IndexBase,
    },
    #[error("{field} has reversed half-open range {start}..{end}")]
    ReversedRange { field: String, start: u64, end: u64 },
    #[error("invalid text position: {0}")]
    InvalidTextPosition(String),
    #[error("invalid bounding box {field}: {reason}")]
    InvalidBoundingBox { field: String, reason: String },
    #[error("confidence must be finite and within [0, 1], got {0}")]
    InvalidConfidence(f64),
    #[error("invalid derived-node reference: {0}")]
    InvalidDerivation(String),
    #[error("component {component} is invalid: {reason}")]
    InvalidComponent { component: usize, reason: String },
    #[error("{0} mixes zero- and one-based indexes")]
    MixedIndexBase(String),
}

fn require_non_empty(value: &str, component: usize, field: &str) -> Result<(), SourceLocatorError> {
    if value.is_empty() {
        return Err(SourceLocatorError::InvalidComponent {
            component,
            reason: format!("{field} cannot be empty"),
        });
    }
    Ok(())
}

fn validate_optional_string(
    value: &Option<String>,
    component: usize,
    field: &str,
) -> Result<(), SourceLocatorError> {
    if value.as_deref().is_some_and(str::is_empty) {
        return Err(SourceLocatorError::InvalidComponent {
            component,
            reason: format!("{field} cannot be empty when present"),
        });
    }
    Ok(())
}

fn valid_json_pointer(pointer: &str) -> bool {
    if pointer.is_empty() {
        return true;
    }
    if !pointer.starts_with('/') {
        return false;
    }
    let bytes = pointer.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'~' {
            if index + 1 >= bytes.len() || !matches!(bytes[index + 1], b'0' | b'1') {
                return false;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    true
}

impl fmt::Display for IndexBase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Zero => "zero",
            Self::One => "one",
        })
    }
}
