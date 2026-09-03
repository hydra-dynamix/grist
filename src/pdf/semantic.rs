//! Confidence-bearing semantic structure inferred from native PDF geometry.

use super::syntax::RawObject;
use super::*;
use crate::core::{
    BoundingBox, CoordinateOrigin, CoordinateUnit, Diagnostic, IndexBase, IndexPosition,
    LocationComponent, LocatorConfidence, SourceLocator,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const PARSER: &str = "grist.pdf";

pub(crate) fn extract_semantic_structure(
    pages: &[PdfPage],
    native: &PdfNativeLayout,
    objects: &BTreeMap<PdfReference, &RawObject>,
    root: Option<PdfReference>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> PdfSemanticStructure {
    let structure_tree = inspect_structure_tree(root, objects, diagnostics);
    let fonts = native
        .fonts
        .iter()
        .map(|font| (font.id.as_str(), font))
        .collect();
    let mut output = Vec::new();
    let mut remaining_graphics = options.max_semantic_graphics;
    for page in pages {
        let layout = native
            .pages
            .iter()
            .find(|value| value.page_index == page.index);
        let mut blocks = layout
            .map(|value| classify_blocks(page, value, &fonts))
            .unwrap_or_default();
        let lists = infer_lists(page, &blocks);
        let graphics = extract_graphics(page, objects, &mut remaining_graphics, diagnostics);
        let tables = layout
            .map(|value| infer_tables(page, value, &blocks, &fonts, &graphics))
            .unwrap_or_default();
        let figures = infer_figures(page, &mut blocks, &graphics, &tables);
        let columns = infer_columns(page, &blocks);
        let reading_order = semantic_reading_order(&blocks, &tables, &figures);
        output.push(PdfPageSemanticStructure {
            page_index: page.index,
            columns,
            blocks,
            lists,
            tables,
            graphics,
            figures,
            reading_order,
        });
    }
    let repeated_regions = infer_repeated_regions(pages, &mut output);
    PdfSemanticStructure {
        structure_tree,
        pages: output,
        repeated_regions,
    }
}

fn inspect_structure_tree(
    root: Option<PdfReference>,
    objects: &BTreeMap<PdfReference, &RawObject>,
    diagnostics: &mut Vec<Diagnostic>,
) -> PdfStructureTreeSummary {
    let Some(root) = root else {
        return PdfStructureTreeSummary {
            root: None,
            status: PdfStructureTreeStatus::Absent,
            element_count: 0,
            confidence: 1.0,
        };
    };
    let valid = objects
        .get(&root)
        .and_then(|value| value.model.value.as_dictionary())
        .is_some_and(|value| {
            value.get("Type").and_then(PdfValue::as_name) == Some("StructTreeRoot")
                && value.contains_key("K")
        });
    let mut visited = HashSet::new();
    let count = valid
        .then(|| count_structure(PdfValue::Reference(root), objects, &mut visited, 0))
        .flatten();
    match count {
        Some(element_count) => PdfStructureTreeSummary {
            root: Some(root),
            status: PdfStructureTreeStatus::Valid,
            element_count,
            confidence: 1.0,
        },
        None => {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.structure_tree.malformed",
                    format!(
                        "structure tree root {root} is missing, cyclic, too deep, or malformed"
                    ),
                )
                .partial(),
            );
            PdfStructureTreeSummary {
                root: Some(root),
                status: PdfStructureTreeStatus::Malformed,
                element_count: 0,
                confidence: 0.0,
            }
        }
    }
}

fn count_structure(
    value: PdfValue,
    objects: &BTreeMap<PdfReference, &RawObject>,
    visited: &mut HashSet<PdfReference>,
    depth: usize,
) -> Option<u64> {
    if depth > 256 {
        return None;
    }
    match value {
        PdfValue::Reference(reference) => {
            if !visited.insert(reference) {
                return None;
            }
            let result = count_structure(
                objects.get(&reference)?.model.value.clone(),
                objects,
                visited,
                depth + 1,
            );
            visited.remove(&reference);
            result
        }
        PdfValue::Array(values) => values.into_iter().try_fold(0u64, |total, value| {
            count_structure(value, objects, visited, depth + 1)
                .and_then(|count| total.checked_add(count))
        }),
        PdfValue::Dictionary(value) => {
            let own = u64::from(
                value.get("Type").and_then(PdfValue::as_name) == Some("StructElem")
                    || value.contains_key("S"),
            );
            value.get("K").cloned().map_or(Some(own), |kids| {
                count_structure(kids, objects, visited, depth + 1)
                    .and_then(|count| own.checked_add(count))
            })
        }
        PdfValue::Integer(_) | PdfValue::Null => Some(0),
        _ => None,
    }
}

fn classify_blocks(
    page: &PdfPage,
    layout: &PdfPageNativeLayout,
    fonts: &BTreeMap<&str, &PdfFont>,
) -> Vec<PdfSemanticBlock> {
    let mut sizes = layout
        .glyphs
        .iter()
        .filter(|glyph| !glyph.text.trim().is_empty())
        .map(|glyph| glyph.font_size)
        .collect::<Vec<_>>();
    sizes.sort_by(f64::total_cmp);
    let body = sizes.get(sizes.len() / 2).copied().unwrap_or(12.0).max(0.1);
    layout
        .blocks
        .iter()
        .map(|block| {
            let glyphs = glyphs_for_block(layout, block);
            let max_size = glyphs
                .iter()
                .map(|glyph| glyph.font_size)
                .fold(body, f64::max);
            let bold = if glyphs.is_empty() {
                0.0
            } else {
                glyphs
                    .iter()
                    .filter(|glyph| {
                        fonts
                            .get(glyph.font_id.as_str())
                            .is_some_and(|font| font.style.bold)
                    })
                    .count() as f64
                    / glyphs.len() as f64
            };
            let text = block.text.trim();
            let (marker, ordered) = list_marker(text);
            let heading_score = (max_size / body - 1.0).clamp(0.0, 0.5) * 1.2
                + bold * 0.3
                + f64::from(text.chars().count() <= 100) * 0.1;
            let (kind, confidence, evidence, heading_level, list_marker) =
                if caption_kind(text).is_some() {
                    (
                        PdfSemanticBlockKind::Caption,
                        0.9,
                        vec![fact(
                            PdfSemanticEvidenceKind::TextPattern,
                            "caption prefix and bounded caption text",
                            0.9,
                            vec![block.index],
                            vec![],
                        )],
                        None,
                        None,
                    )
                } else if let Some(marker) = marker {
                    (
                        PdfSemanticBlockKind::ListItem,
                        0.92,
                        vec![fact(
                            PdfSemanticEvidenceKind::TextPattern,
                            if ordered {
                                "ordered list marker"
                            } else {
                                "bullet list marker"
                            },
                            0.92,
                            vec![block.index],
                            vec![],
                        )],
                        None,
                        Some(marker),
                    )
                } else if heading_score >= 0.28 && text.chars().count() <= 160 {
                    let level = if max_size >= body * 1.8 {
                        1
                    } else if max_size >= body * 1.4 {
                        2
                    } else {
                        3
                    };
                    let mut evidence = vec![fact(
                        PdfSemanticEvidenceKind::FontScale,
                        "font size exceeds the page median",
                        (0.65 + (max_size / body - 1.0).min(0.3)).min(0.95),
                        vec![block.index],
                        vec![],
                    )];
                    if bold >= 0.5 {
                        evidence.push(fact(
                            PdfSemanticEvidenceKind::FontStyle,
                            "most glyphs use a bold font",
                            0.85,
                            vec![block.index],
                            vec![],
                        ));
                    }
                    (
                        PdfSemanticBlockKind::Heading,
                        heading_score.min(0.95),
                        evidence,
                        Some(level),
                        None,
                    )
                } else {
                    (
                        PdfSemanticBlockKind::Paragraph,
                        0.78,
                        vec![fact(
                            PdfSemanticEvidenceKind::GeometricAlignment,
                            "contiguous native lines form a text block",
                            block.confidence,
                            vec![block.index],
                            vec![],
                        )],
                        None,
                        None,
                    )
                };
            PdfSemanticBlock {
                index: block.index,
                native_block_index: block.index,
                kind,
                heading_level,
                list_marker,
                text: block.text.clone(),
                bbox: block.bbox,
                confidence,
                evidence,
                locator: locator(page, block.bbox, confidence),
            }
        })
        .collect()
}

fn glyphs_for_block<'a>(
    layout: &'a PdfPageNativeLayout,
    block: &PdfTextBlock,
) -> Vec<&'a PdfGlyph> {
    let ranges = layout
        .tokens
        .iter()
        .filter(|token| token.index >= block.token_start && token.index < block.token_end)
        .map(|token| (token.glyph_start, token.glyph_end))
        .collect::<Vec<_>>();
    layout
        .glyphs
        .iter()
        .filter(|glyph| {
            ranges
                .iter()
                .any(|(start, end)| glyph.index >= *start && glyph.index < *end)
        })
        .collect()
}

fn list_marker(text: &str) -> (Option<String>, bool) {
    let first = text.split_whitespace().next().unwrap_or_default();
    if ["-", "*", "+", "•", "‣", "◦", "▪"].contains(&first) {
        return (Some(first.into()), false);
    }
    let core = first.trim_end_matches(['.', ')', ':']);
    let ordered = !core.is_empty()
        && first.len() > core.len()
        && (core.chars().all(|value| value.is_ascii_digit())
            || (core.len() <= 4 && core.chars().all(|value| "ivxlcdmIVXLCDM".contains(value)))
            || (core.len() == 1 && core.chars().all(|value| value.is_ascii_alphabetic())));
    (ordered.then(|| first.into()), ordered)
}

fn caption_kind(text: &str) -> Option<PdfSemanticReadingItemKind> {
    let value = text.trim_start().to_ascii_lowercase();
    if value.starts_with("figure ") || value.starts_with("fig. ") || value.starts_with("fig ") {
        Some(PdfSemanticReadingItemKind::Figure)
    } else if value.starts_with("table ") {
        Some(PdfSemanticReadingItemKind::Table)
    } else {
        None
    }
}

fn infer_lists(page: &PdfPage, blocks: &[PdfSemanticBlock]) -> Vec<PdfList> {
    let mut groups = Vec::<Vec<&PdfSemanticBlock>>::new();
    for block in blocks
        .iter()
        .filter(|block| block.kind == PdfSemanticBlockKind::ListItem)
    {
        let append = groups
            .last()
            .and_then(|group| group.last())
            .is_some_and(|previous| {
                previous.index + 1 == block.index
                    && (previous.bbox.x - block.bbox.x).abs()
                        <= previous.bbox.height.max(block.bbox.height) * 1.5
            });
        if append {
            groups.last_mut().unwrap().push(block);
        } else {
            groups.push(vec![block]);
        }
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(index, group)| {
            let bbox = union_many(group.iter().map(|block| block.bbox)).unwrap();
            let confidence = group
                .iter()
                .map(|block| block.confidence)
                .fold(1.0, f64::min)
                * 0.96;
            let indices = group.iter().map(|block| block.index).collect::<Vec<_>>();
            PdfList {
                index: index as u64 + 1,
                item_block_indices: indices.clone(),
                ordered: group.iter().all(|block| {
                    block.list_marker.as_deref().is_some_and(|marker| {
                        marker
                            .chars()
                            .next()
                            .is_some_and(|value| value.is_ascii_alphanumeric())
                    })
                }),
                confidence,
                evidence: vec![fact(
                    PdfSemanticEvidenceKind::GeometricAlignment,
                    "consecutive marked blocks share indentation",
                    confidence,
                    indices,
                    vec![],
                )],
                locator: locator(page, bbox, confidence),
            }
        })
        .collect()
}

fn infer_columns(page: &PdfPage, blocks: &[PdfSemanticBlock]) -> Vec<PdfColumn> {
    let mut groups = Vec::<(BoundingBox, Vec<u64>)>::new();
    let mut ordered = blocks.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.bbox.x.total_cmp(&b.bbox.x));
    for block in ordered {
        let found = groups.iter().position(|(bbox, _)| {
            let overlap = (right(*bbox).min(right(block.bbox)) - bbox.x.max(block.bbox.x)).max(0.0);
            overlap / bbox.width.min(block.bbox.width).max(1.0) >= 0.35
        });
        if let Some(index) = found {
            groups[index].0 = union(groups[index].0, block.bbox);
            groups[index].1.push(block.index);
        } else {
            groups.push((block.bbox, vec![block.index]));
        }
    }
    let confidence = if groups.len() > 1 { 0.82 } else { 0.9 };
    groups
        .into_iter()
        .enumerate()
        .map(|(index, (bbox, indices))| PdfColumn {
            index: index as u64 + 1,
            block_indices: indices.clone(),
            bbox,
            confidence,
            evidence: vec![fact(
                PdfSemanticEvidenceKind::ColumnSeparation,
                "non-overlapping horizontal block bands",
                confidence,
                indices,
                vec![],
            )],
            locator: locator(page, bbox, confidence),
        })
        .collect()
}

#[derive(Clone)]
struct CellSeed {
    text: String,
    bbox: BoundingBox,
    token_start: u64,
    token_end: u64,
}

fn infer_tables(
    page: &PdfPage,
    layout: &PdfPageNativeLayout,
    blocks: &[PdfSemanticBlock],
    fonts: &BTreeMap<&str, &PdfFont>,
    graphics: &[PdfGraphicObject],
) -> Vec<PdfTable> {
    let ruled = graphics
        .iter()
        .flat_map(|item| &item.segments)
        .filter(|line| axis_aligned(**line))
        .count()
        >= 6;
    let mut candidates = Vec::<(Vec<CellSeed>, BoundingBox)>::new();
    let mut row_groups = Vec::<Vec<&PdfTextLine>>::new();
    let mut ordered_lines = layout.lines.iter().collect::<Vec<_>>();
    ordered_lines.sort_by(|a, b| top(b.bbox).total_cmp(&top(a.bbox)));
    for line in ordered_lines {
        if let Some(group) = row_groups.iter_mut().find(|group| {
            let first = group[0];
            ((first.bbox.y + first.bbox.height / 2.0) - (line.bbox.y + line.bbox.height / 2.0))
                .abs()
                <= first.bbox.height.max(line.bbox.height) * 0.55
        }) {
            group.push(line);
        } else {
            row_groups.push(vec![line]);
        }
    }
    for row_lines in row_groups {
        let mut tokens = row_lines
            .iter()
            .flat_map(|line| {
                layout
                    .tokens
                    .iter()
                    .filter(|token| token.index >= line.token_start && token.index < line.token_end)
            })
            .collect::<Vec<_>>();
        tokens.sort_by(|a, b| a.bbox.x.total_cmp(&b.bbox.x));
        let mut cells = Vec::<CellSeed>::new();
        for token in tokens {
            let split = cells.last().is_some_and(|previous| {
                token.bbox.x - right(previous.bbox)
                    > token.bbox.height.max(previous.bbox.height) * 1.25
            });
            if split || cells.is_empty() {
                cells.push(CellSeed {
                    text: token.text.clone(),
                    bbox: token.bbox,
                    token_start: token.index,
                    token_end: token.index + 1,
                });
            } else {
                let cell = cells.last_mut().unwrap();
                cell.text.push(' ');
                cell.text.push_str(&token.text);
                cell.bbox = union(cell.bbox, token.bbox);
                cell.token_end = token.index + 1;
            }
        }
        if cells.len() >= 2
            || (ruled
                && !cells.is_empty()
                && within_ruling_area(
                    graphics,
                    union_many(row_lines.iter().map(|line| line.bbox)).unwrap(),
                ))
        {
            candidates.push((
                cells,
                union_many(row_lines.iter().map(|line| line.bbox)).unwrap(),
            ));
        }
    }
    let mut groups = Vec::<Vec<(Vec<CellSeed>, BoundingBox)>>::new();
    for row in candidates {
        let append = groups
            .last()
            .and_then(|group| group.last())
            .is_some_and(|previous| {
                (previous.1.y - top(row.1)).abs() <= previous.1.height.max(row.1.height) * 3.0
                    && (ruled || aligned_cells(&previous.0, &row.0))
            });
        if append {
            groups.last_mut().unwrap().push(row);
        } else {
            groups.push(vec![row]);
        }
    }
    groups
        .into_iter()
        .filter(|group| group.len() >= 2)
        .enumerate()
        .map(|(table_index, group)| {
            let mut anchors = group
                .iter()
                .flat_map(|(cells, _)| cells.iter().map(|cell| cell.bbox.x))
                .collect::<Vec<_>>();
            anchors.sort_by(f64::total_cmp);
            anchors.dedup_by(|a, b| (*a - *b).abs() <= 3.0);
            let mut column_count = anchors.len().max(
                group
                    .iter()
                    .map(|(cells, _)| cells.len())
                    .max()
                    .unwrap_or(0),
            );
            let bbox = union_many(group.iter().map(|(_, bbox)| *bbox)).unwrap();
            let confidence = if ruled { 0.94 } else { 0.82 };
            let mut rows: Vec<PdfTableRow> = group
                .iter()
                .enumerate()
                .map(|(row_index, (cells, row_bbox))| {
                    let cells = cells
                        .iter()
                        .enumerate()
                        .map(|(ordinal, cell)| {
                            let column = anchors
                                .iter()
                                .enumerate()
                                .min_by(|(_, a), (_, b)| {
                                    (cell.bbox.x - **a)
                                        .abs()
                                        .total_cmp(&(cell.bbox.x - **b).abs())
                                })
                                .map_or(ordinal, |(index, _)| index);
                            let glyphs = layout
                                .glyphs
                                .iter()
                                .filter(|glyph| {
                                    layout
                                        .tokens
                                        .iter()
                                        .filter(|token| {
                                            token.index >= cell.token_start
                                                && token.index < cell.token_end
                                        })
                                        .any(|token| {
                                            glyph.index >= token.glyph_start
                                                && glyph.index < token.glyph_end
                                        })
                                })
                                .collect::<Vec<_>>();
                            let bold = !glyphs.is_empty()
                                && glyphs
                                    .iter()
                                    .filter(|glyph| {
                                        fonts
                                            .get(glyph.font_id.as_str())
                                            .is_some_and(|font| font.style.bold)
                                    })
                                    .count()
                                    * 2
                                    >= glyphs.len();
                            let header_confidence = if bold {
                                0.92
                            } else if row_index == 0 {
                                0.68
                            } else {
                                0.15
                            };
                            PdfTableCell {
                                row: row_index as u64 + 1,
                                column: column as u64 + 1,
                                row_span: 1,
                                column_span: 1,
                                text: cell.text.clone(),
                                header_candidate: header_confidence >= 0.6,
                                header_confidence,
                                bbox: cell.bbox,
                                confidence,
                                evidence: vec![fact(
                                    if ruled {
                                        PdfSemanticEvidenceKind::RulingLines
                                    } else {
                                        PdfSemanticEvidenceKind::GeometricAlignment
                                    },
                                    "cell text occupies a stable row and column band",
                                    confidence,
                                    block_indexes(blocks, cell.bbox),
                                    vec![],
                                )],
                                locator: locator(page, cell.bbox, confidence),
                            }
                        })
                        .collect();
                    PdfTableRow {
                        index: row_index as u64 + 1,
                        cells,
                        bbox: *row_bbox,
                        locator: locator(page, *row_bbox, confidence),
                    }
                })
                .collect();
            column_count = column_count.max(apply_ruling_spans(&mut rows, graphics, bbox));
            let source_blocks = block_indexes(blocks, bbox);
            PdfTable {
                index: table_index as u64 + 1,
                rows,
                column_count: column_count as u64,
                caption_block_index: nearest_caption(
                    blocks,
                    bbox,
                    PdfSemanticReadingItemKind::Table,
                ),
                bbox,
                confidence,
                evidence: vec![fact(
                    if ruled {
                        PdfSemanticEvidenceKind::RulingLines
                    } else {
                        PdfSemanticEvidenceKind::GeometricAlignment
                    },
                    if ruled {
                        "axis-aligned rulings and aligned text define a table grid"
                    } else {
                        "multiple text rows share stable column anchors"
                    },
                    confidence,
                    source_blocks,
                    vec![],
                )],
                locator: locator(page, bbox, confidence),
            }
        })
        .collect()
}

fn aligned_cells(left: &[CellSeed], right_cells: &[CellSeed]) -> bool {
    let matched = left
        .iter()
        .filter(|cell| {
            right_cells.iter().any(|other| {
                (cell.bbox.x - other.bbox.x).abs() <= cell.bbox.height.max(other.bbox.height) * 1.5
            })
        })
        .count();
    matched >= 2 || (matched >= 1 && left.len() == right_cells.len())
}

fn apply_ruling_spans(
    rows: &mut [PdfTableRow],
    graphics: &[PdfGraphicObject],
    bbox: BoundingBox,
) -> usize {
    let segments = graphics
        .iter()
        .flat_map(|graphic| graphic.segments.iter())
        .copied()
        .filter(|line| axis_aligned(*line) && line_bbox_near(*line, bbox))
        .collect::<Vec<_>>();
    let vertical = segments
        .iter()
        .copied()
        .filter(|line| (line.start[0] - line.end[0]).abs() <= 0.5)
        .collect::<Vec<_>>();
    let horizontal = segments
        .iter()
        .copied()
        .filter(|line| (line.start[1] - line.end[1]).abs() <= 0.5)
        .collect::<Vec<_>>();
    let mut xs = vertical
        .iter()
        .map(|line| (line.start[0] + line.end[0]) / 2.0)
        .collect::<Vec<_>>();
    xs.sort_by(f64::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() <= 1.0);
    let mut ys = horizontal
        .iter()
        .map(|line| (line.start[1] + line.end[1]) / 2.0)
        .collect::<Vec<_>>();
    ys.sort_by(|a, b| b.total_cmp(a));
    ys.dedup_by(|a, b| (*a - *b).abs() <= 1.0);
    if xs.len() < 3 || ys.len() < 3 {
        return 0;
    }
    for row in rows.iter_mut() {
        let row_y = row.bbox.y + row.bbox.height / 2.0;
        for cell in &mut row.cells {
            let center_x = cell.bbox.x + cell.bbox.width / 2.0;
            let column = xs
                .windows(2)
                .position(|pair| center_x >= pair[0] - 1.0 && center_x <= pair[1] + 1.0)
                .unwrap_or(cell.column.saturating_sub(1) as usize);
            cell.column = column as u64 + 1;
            let mut column_span = 1usize;
            for boundary in xs
                .iter()
                .skip(column + 1)
                .take(xs.len().saturating_sub(column + 2))
            {
                if vertical.iter().any(|line| {
                    (line.start[0] - *boundary).abs() <= 1.0
                        && row_y >= line.start[1].min(line.end[1]) - 1.0
                        && row_y <= line.start[1].max(line.end[1]) + 1.0
                }) {
                    break;
                }
                column_span += 1;
            }
            let row_index = ys
                .windows(2)
                .position(|pair| row_y <= pair[0] + 1.0 && row_y >= pair[1] - 1.0)
                .unwrap_or(cell.row.saturating_sub(1) as usize);
            let mut row_span = 1usize;
            for boundary in ys
                .iter()
                .skip(row_index + 1)
                .take(ys.len().saturating_sub(row_index + 2))
            {
                if horizontal.iter().any(|line| {
                    (line.start[1] - *boundary).abs() <= 1.0
                        && center_x >= line.start[0].min(line.end[0]) - 1.0
                        && center_x <= line.start[0].max(line.end[0]) + 1.0
                }) {
                    break;
                }
                row_span += 1;
            }
            cell.column_span = column_span as u64;
            cell.row_span = row_span as u64;
            if column_span > 1 || row_span > 1 {
                cell.evidence.push(fact(
                    PdfSemanticEvidenceKind::RulingLines,
                    "missing interior ruling across this grid band supports a merged-cell span",
                    0.9,
                    Vec::new(),
                    Vec::new(),
                ));
                cell.confidence = cell.confidence.min(0.9);
            }
        }
    }
    xs.len() - 1
}

fn line_bbox_near(line: PdfLineSegment, bbox: BoundingBox) -> bool {
    let min_x = line.start[0].min(line.end[0]);
    let max_x = line.start[0].max(line.end[0]);
    let min_y = line.start[1].min(line.end[1]);
    let max_y = line.start[1].max(line.end[1]);
    max_x >= bbox.x - 100.0
        && min_x <= right(bbox) + 100.0
        && max_y >= bbox.y - 100.0
        && min_y <= top(bbox) + 100.0
}

fn within_ruling_area(graphics: &[PdfGraphicObject], bbox: BoundingBox) -> bool {
    let area = points_bbox(
        graphics
            .iter()
            .flat_map(|graphic| graphic.segments.iter())
            .filter(|line| axis_aligned(**line))
            .flat_map(|line| [line.start, line.end]),
    );
    area.is_some_and(|area| {
        let center = [bbox.x + bbox.width / 2.0, bbox.y + bbox.height / 2.0];
        center[0] >= area.x - 2.0
            && center[0] <= right(area) + 2.0
            && center[1] >= area.y - 2.0
            && center[1] <= top(area) + 2.0
    })
}

fn infer_figures(
    page: &PdfPage,
    blocks: &mut [PdfSemanticBlock],
    graphics: &[PdfGraphicObject],
    tables: &[PdfTable],
) -> Vec<PdfFigure> {
    graphics
        .iter()
        .filter(|graphic| {
            graphic.kind != PdfGraphicObjectKind::VectorPath
                || (graphic.bbox.width > 8.0
                    && graphic.bbox.height > 8.0
                    && !tables
                        .iter()
                        .any(|table| overlap(table.bbox, graphic.bbox) > 0.5))
        })
        .enumerate()
        .map(|(index, graphic)| {
            let caption = nearest_caption(blocks, graphic.bbox, PdfSemanticReadingItemKind::Figure);
            if let Some(block) =
                caption.and_then(|id| blocks.iter_mut().find(|block| block.index == id))
            {
                block.kind = PdfSemanticBlockKind::Caption;
            }
            let confidence = if caption.is_some() {
                0.94
            } else {
                graphic.confidence * 0.82
            };
            PdfFigure {
                index: index as u64 + 1,
                graphic_indices: vec![graphic.index],
                caption_block_index: caption,
                bbox: graphic.bbox,
                confidence,
                evidence: vec![fact(
                    if caption.is_some() {
                        PdfSemanticEvidenceKind::CaptionProximity
                    } else {
                        PdfSemanticEvidenceKind::XObjectInvocation
                    },
                    if caption.is_some() {
                        "caption prefix is geometrically adjacent to a graphic object"
                    } else {
                        "painted raster or substantial vector object forms a figure candidate"
                    },
                    confidence,
                    caption.into_iter().collect(),
                    vec![graphic.source_object],
                )],
                locator: locator(page, graphic.bbox, confidence),
            }
        })
        .collect()
}

fn nearest_caption(
    blocks: &[PdfSemanticBlock],
    bbox: BoundingBox,
    kind: PdfSemanticReadingItemKind,
) -> Option<u64> {
    blocks
        .iter()
        .filter(|block| caption_kind(&block.text) == Some(kind))
        .filter_map(|block| {
            let distance = if block.bbox.y >= top(bbox) {
                block.bbox.y - top(bbox)
            } else if bbox.y >= top(block.bbox) {
                bbox.y - top(block.bbox)
            } else {
                0.0
            };
            (distance <= block.bbox.height.max(10.0) * 4.0).then_some((block.index, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

fn semantic_reading_order(
    blocks: &[PdfSemanticBlock],
    tables: &[PdfTable],
    figures: &[PdfFigure],
) -> Vec<PdfSemanticReadingItem> {
    let table_blocks = tables
        .iter()
        .flat_map(|table| table.evidence.iter())
        .flat_map(|value| value.source_blocks.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut items = blocks
        .iter()
        .filter(|block| !table_blocks.contains(&block.index))
        .map(|block| PdfSemanticReadingItem {
            kind: PdfSemanticReadingItemKind::Block,
            index: block.index,
            bbox: block.bbox,
            confidence: block.confidence,
        })
        .collect::<Vec<_>>();
    items.extend(tables.iter().map(|table| PdfSemanticReadingItem {
        kind: PdfSemanticReadingItemKind::Table,
        index: table.index,
        bbox: table.bbox,
        confidence: table.confidence,
    }));
    items.extend(figures.iter().map(|figure| PdfSemanticReadingItem {
        kind: PdfSemanticReadingItemKind::Figure,
        index: figure.index,
        bbox: figure.bbox,
        confidence: figure.confidence,
    }));
    items.sort_by(|a, b| {
        top(b.bbox)
            .total_cmp(&top(a.bbox))
            .then_with(|| a.bbox.x.total_cmp(&b.bbox.x))
            .then_with(|| reading_rank(a.kind).cmp(&reading_rank(b.kind)))
            .then_with(|| a.index.cmp(&b.index))
    });
    items
}

fn reading_rank(kind: PdfSemanticReadingItemKind) -> u8 {
    match kind {
        PdfSemanticReadingItemKind::Block => 0,
        PdfSemanticReadingItemKind::Table => 1,
        PdfSemanticReadingItemKind::Figure => 2,
    }
}

fn infer_repeated_regions(
    pages: &[PdfPage],
    semantic: &mut [PdfPageSemanticStructure],
) -> Vec<PdfRepeatedRegion> {
    let heights = pages
        .iter()
        .map(|page| (page.index, page.height_points))
        .collect::<BTreeMap<_, _>>();
    let mut patterns =
        BTreeMap::<(String, PdfRepeatedRegionKind), Vec<PdfRepeatedRegionOccurrence>>::new();
    for page in semantic.iter() {
        let height = heights.get(&page.page_index).copied().unwrap_or(1.0);
        for block in &page.blocks {
            let near_top = top(block.bbox) >= height * 0.86;
            let near_bottom = block.bbox.y <= height * 0.14;
            if !near_top && !near_bottom {
                continue;
            }
            let pattern = normalized_pattern(&block.text);
            if pattern.is_empty() {
                continue;
            }
            let kind = if page_number(&block.text) {
                PdfRepeatedRegionKind::PageNumber
            } else if near_top {
                PdfRepeatedRegionKind::Header
            } else {
                PdfRepeatedRegionKind::Footer
            };
            patterns
                .entry((pattern, kind))
                .or_default()
                .push(PdfRepeatedRegionOccurrence {
                    page_index: page.page_index,
                    block_index: block.index,
                    text: block.text.clone(),
                    locator: block.locator.clone(),
                });
        }
    }
    let regions = patterns
        .into_iter()
        .filter(|(_, values)| values.len() >= 2)
        .enumerate()
        .map(|(index, ((pattern, kind), occurrences))| {
            let confidence =
                (0.72 + occurrences.len() as f64 / semantic.len().max(1) as f64 * 0.25).min(0.97);
            PdfRepeatedRegion {
                index: index as u64 + 1,
                kind,
                normalized_pattern: pattern,
                evidence: vec![fact(
                    PdfSemanticEvidenceKind::RepeatedPosition,
                    "normalized text recurs in a consistent page margin",
                    confidence,
                    occurrences.iter().map(|value| value.block_index).collect(),
                    vec![],
                )],
                occurrences,
                confidence,
            }
        })
        .collect::<Vec<_>>();
    for region in &regions {
        for occurrence in &region.occurrences {
            if let Some(block) = semantic
                .iter_mut()
                .find(|page| page.page_index == occurrence.page_index)
                .and_then(|page| {
                    page.blocks
                        .iter_mut()
                        .find(|block| block.index == occurrence.block_index)
                })
            {
                block.kind = match region.kind {
                    PdfRepeatedRegionKind::Header => PdfSemanticBlockKind::Header,
                    PdfRepeatedRegionKind::Footer => PdfSemanticBlockKind::Footer,
                    PdfRepeatedRegionKind::PageNumber => PdfSemanticBlockKind::PageNumber,
                };
                block.confidence = block.confidence.max(region.confidence);
                block.evidence.extend(region.evidence.clone());
            }
        }
    }
    regions
}

fn normalized_pattern(text: &str) -> String {
    let mut output = String::new();
    let mut digits = false;
    for value in text.trim().to_lowercase().chars() {
        if value.is_ascii_digit() {
            if !digits {
                output.push('#');
                digits = true;
            }
        } else {
            digits = false;
            if value.is_whitespace() {
                if !output.ends_with(' ') {
                    output.push(' ');
                }
            } else {
                output.push(value);
            }
        }
    }
    output.trim().into()
}

fn page_number(text: &str) -> bool {
    let value = text.trim().trim_matches(['-', '–', '—', '(', ')']);
    !value.is_empty()
        && value.len() <= 12
        && (value.chars().all(|c| c.is_ascii_digit())
            || value.chars().all(|c| "ivxlcdmIVXLCDM".contains(c))
            || value.to_ascii_lowercase().starts_with("page "))
}

#[derive(Clone, Copy)]
struct Matrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}
impl Matrix {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
    fn apply(self, p: [f64; 2]) -> [f64; 2] {
        [
            self.a * p[0] + self.c * p[1] + self.e,
            self.b * p[0] + self.d * p[1] + self.f,
        ]
    }
    fn concat(self, o: Self) -> Self {
        Self {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }
}

enum GraphicToken {
    Number(f64),
    Name(String),
    Word(String),
}

fn extract_graphics(
    page: &PdfPage,
    objects: &BTreeMap<PdfReference, &RawObject>,
    remaining: &mut u64,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PdfGraphicObject> {
    let resources = inherited_dictionary(page.object, "Resources", objects);
    let xobjects = resources
        .as_ref()
        .and_then(|value| value.get("XObject"))
        .and_then(|value| resolve(value, objects))
        .and_then(PdfValue::as_dictionary);
    let streams = page_streams(page, objects);
    let mut output = Vec::new();
    let mut unsupported = BTreeSet::new();
    for (source, bytes) in streams {
        let mut operands = Vec::new();
        let mut ctm = Matrix::identity();
        let mut stack = Vec::new();
        let mut inline_image = false;
        let mut current = None;
        let mut path = Vec::<PdfLineSegment>::new();
        for token in lex_graphics(bytes) {
            match token {
                GraphicToken::Number(_) | GraphicToken::Name(_) => operands.push(token),
                GraphicToken::Word(operator) => {
                    match operator.as_str() {
                        "q" => stack.push(ctm),
                        "Q" => ctm = stack.pop().unwrap_or_else(Matrix::identity),
                        "cm" => {
                            if let Some(v) = numbers::<6>(&operands) {
                                ctm = ctm.concat(Matrix {
                                    a: v[0],
                                    b: v[1],
                                    c: v[2],
                                    d: v[3],
                                    e: v[4],
                                    f: v[5],
                                });
                            }
                        }
                        "m" => {
                            if let Some(v) = numbers::<2>(&operands) {
                                current = Some(ctm.apply(v));
                            }
                        }
                        "l" => {
                            if let (Some(start), Some(v)) = (current, numbers::<2>(&operands)) {
                                let end = ctm.apply(v);
                                path.push(PdfLineSegment { start, end });
                                current = Some(end);
                            }
                        }
                        "re" => {
                            if let Some(v) = numbers::<4>(&operands) {
                                let p = [
                                    ctm.apply([v[0], v[1]]),
                                    ctm.apply([v[0] + v[2], v[1]]),
                                    ctm.apply([v[0] + v[2], v[1] + v[3]]),
                                    ctm.apply([v[0], v[1] + v[3]]),
                                ];
                                for i in 0..4 {
                                    path.push(PdfLineSegment {
                                        start: p[i],
                                        end: p[(i + 1) % 4],
                                    });
                                }
                                current = Some(p[0]);
                            }
                        }
                        "h" => {
                            if let (Some(first), Some(last)) =
                                (path.first().map(|line| line.start), current)
                            {
                                path.push(PdfLineSegment {
                                    start: last,
                                    end: first,
                                });
                                current = Some(first);
                            }
                        }
                        "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                            if !path.is_empty() && *remaining > 0 {
                                output.push(vector_graphic(
                                    page,
                                    source,
                                    &path,
                                    output.len() as u64 + 1,
                                ));
                                *remaining -= 1;
                            }
                            path.clear();
                            current = None;
                        }
                        "n" => {
                            path.clear();
                            current = None;
                        }
                        "Do" => {
                            if let Some(name) =
                                operands.iter().rev().find_map(|value| match value {
                                    GraphicToken::Name(name) => Some(name.as_str()),
                                    _ => None,
                                })
                                && *remaining > 0
                                && let Some(graphic) = xobject_graphic(
                                    page,
                                    source,
                                    name,
                                    ctm,
                                    xobjects,
                                    objects,
                                    output.len() as u64 + 1,
                                )
                            {
                                output.push(graphic);
                                *remaining -= 1;
                            }
                        }
                        "BI" => inline_image = true,
                        "ID" if inline_image && *remaining > 0 => {
                            let width = named_integer(&operands, "W")
                                .or_else(|| named_integer(&operands, "Width"));
                            let height = named_integer(&operands, "H")
                                .or_else(|| named_integer(&operands, "Height"));
                            let bbox = points_bbox([
                                ctm.apply([0.0, 0.0]),
                                ctm.apply([1.0, 0.0]),
                                ctm.apply([1.0, 1.0]),
                                ctm.apply([0.0, 1.0]),
                            ])
                            .unwrap();
                            output.push(PdfGraphicObject {
                                index: output.len() as u64 + 1,
                                kind: PdfGraphicObjectKind::RasterImage,
                                source_object: source,
                                resource_name: Some("inline_image".into()),
                                pixel_width: width,
                                pixel_height: height,
                                bbox,
                                segments: Vec::new(),
                                confidence: 0.75,
                                evidence: vec![fact(
                                    PdfSemanticEvidenceKind::XObjectInvocation,
                                    "inline-image BI/ID/EI sequence under the current transform",
                                    0.75,
                                    vec![],
                                    vec![source],
                                )],
                                locator: locator(page, bbox, 0.75),
                            });
                            *remaining -= 1;
                            inline_image = false;
                        }
                        "sh" => {
                            unsupported.insert(operator.clone());
                        }
                        _ => {}
                    }
                    operands.clear();
                }
            }
            if *remaining == 0 {
                break;
            }
        }
    }
    if *remaining == 0 {
        diagnostics.push(
            Diagnostic::budget_exhausted(PARSER, "PDF semantic graphic-object budget reached")
                .with_locator(page.locator.clone()),
        );
    }
    if !unsupported.is_empty() {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.semantic.graphics_loss",
                format!(
                    "unsupported inline-image or shading operators on page {}: {}",
                    page.index,
                    unsupported.into_iter().collect::<Vec<_>>().join(", ")
                ),
            )
            .partial()
            .with_locator(page.locator.clone()),
        );
    }
    output
}

fn vector_graphic(
    page: &PdfPage,
    source: PdfReference,
    segments: &[PdfLineSegment],
    index: u64,
) -> PdfGraphicObject {
    let bbox = points_bbox(segments.iter().flat_map(|line| [line.start, line.end])).unwrap();
    PdfGraphicObject {
        index,
        kind: PdfGraphicObjectKind::VectorPath,
        source_object: source,
        resource_name: None,
        pixel_width: None,
        pixel_height: None,
        bbox,
        segments: segments.to_vec(),
        confidence: 0.96,
        evidence: vec![fact(
            PdfSemanticEvidenceKind::RulingLines,
            "painted PDF path with transformed line geometry",
            0.96,
            vec![],
            vec![source],
        )],
        locator: locator(page, bbox, 0.96),
    }
}

fn xobject_graphic(
    page: &PdfPage,
    source: PdfReference,
    name: &str,
    ctm: Matrix,
    resources: Option<&BTreeMap<String, PdfValue>>,
    objects: &BTreeMap<PdfReference, &RawObject>,
    index: u64,
) -> Option<PdfGraphicObject> {
    let reference = resources?.get(name)?.as_reference()?;
    let dictionary = objects.get(&reference)?.model.value.as_dictionary()?;
    let (kind, local, width, height) =
        match dictionary.get("Subtype").and_then(PdfValue::as_name)? {
            "Image" => (
                PdfGraphicObjectKind::RasterImage,
                [0.0, 0.0, 1.0, 1.0],
                integer(dictionary.get("Width")),
                integer(dictionary.get("Height")),
            ),
            "Form" => (
                PdfGraphicObjectKind::FormXObject,
                rect_values(dictionary.get("BBox")).unwrap_or([0.0, 0.0, 1.0, 1.0]),
                None,
                None,
            ),
            _ => return None,
        };
    let bbox = points_bbox([
        ctm.apply([local[0], local[1]]),
        ctm.apply([local[2], local[1]]),
        ctm.apply([local[2], local[3]]),
        ctm.apply([local[0], local[3]]),
    ])
    .unwrap();
    Some(PdfGraphicObject {
        index,
        kind,
        source_object: reference,
        resource_name: Some(name.into()),
        pixel_width: width,
        pixel_height: height,
        bbox,
        segments: Vec::new(),
        confidence: 0.98,
        evidence: vec![fact(
            PdfSemanticEvidenceKind::XObjectInvocation,
            "content-stream Do operator invokes a typed XObject under the current transform",
            0.98,
            vec![],
            vec![source, reference],
        )],
        locator: locator(page, bbox, 0.98),
    })
}

fn lex_graphics(bytes: &[u8]) -> Vec<GraphicToken> {
    let mut output = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        if bytes[cursor] == b'%' {
            while cursor < bytes.len() && !matches!(bytes[cursor], b'\r' | b'\n') {
                cursor += 1;
            }
            continue;
        }
        if bytes[cursor] == b'(' {
            skip_string(bytes, &mut cursor);
            continue;
        }
        if bytes[cursor] == b'<' {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor] != b'>' {
                cursor += 1;
            }
            cursor = (cursor + 1).min(bytes.len());
            continue;
        }
        let start = cursor;
        if bytes[cursor] == b'/' {
            cursor += 1;
            while cursor < bytes.len() && !delimiter(bytes[cursor]) {
                cursor += 1;
            }
            output.push(GraphicToken::Name(
                String::from_utf8_lossy(&bytes[start + 1..cursor]).into(),
            ));
            continue;
        }
        while cursor < bytes.len() && !delimiter(bytes[cursor]) {
            cursor += 1;
        }
        if cursor == start {
            cursor += 1;
            continue;
        }
        let value = String::from_utf8_lossy(&bytes[start..cursor]);
        if let Ok(number) = value.parse() {
            output.push(GraphicToken::Number(number));
        } else {
            output.push(GraphicToken::Word(value.into()));
        }
    }
    output
}

fn delimiter(value: u8) -> bool {
    value.is_ascii_whitespace() || b"[]<>()/".contains(&value)
}
fn skip_string(bytes: &[u8], cursor: &mut usize) {
    let mut depth = 0usize;
    while *cursor < bytes.len() {
        let value = bytes[*cursor];
        *cursor += 1;
        if value == b'\\' {
            *cursor = (*cursor + 1).min(bytes.len());
        } else if value == b'(' {
            depth += 1;
        } else if value == b')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
}
fn numbers<const N: usize>(values: &[GraphicToken]) -> Option<[f64; N]> {
    let values = values
        .iter()
        .filter_map(|value| match value {
            GraphicToken::Number(value) => Some(*value),
            _ => None,
        })
        .collect::<Vec<_>>();
    values.get(values.len().checked_sub(N)?..)?.try_into().ok()
}
fn named_integer(values: &[GraphicToken], name: &str) -> Option<u64> {
    values
        .windows(2)
        .find_map(|pair| match (&pair[0], &pair[1]) {
            (GraphicToken::Name(candidate), GraphicToken::Number(value))
                if candidate == name && *value >= 0.0 =>
            {
                Some(*value as u64)
            }
            _ => None,
        })
}

fn inherited_dictionary(
    reference: PdfReference,
    key: &str,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> Option<BTreeMap<String, PdfValue>> {
    let mut current = Some(reference);
    let mut visited = HashSet::new();
    while let Some(reference) = current {
        if !visited.insert(reference) {
            return None;
        }
        let dictionary = objects.get(&reference)?.model.value.as_dictionary()?;
        if let Some(value) = dictionary
            .get(key)
            .and_then(|value| resolve(value, objects))
            .and_then(PdfValue::as_dictionary)
        {
            return Some(value.clone());
        }
        current = dictionary.get("Parent").and_then(PdfValue::as_reference);
    }
    None
}
fn resolve<'a>(
    value: &'a PdfValue,
    objects: &'a BTreeMap<PdfReference, &RawObject>,
) -> Option<&'a PdfValue> {
    match value {
        PdfValue::Reference(reference) => objects.get(reference).map(|value| &value.model.value),
        _ => Some(value),
    }
}
fn page_streams<'a>(
    page: &PdfPage,
    objects: &'a BTreeMap<PdfReference, &RawObject>,
) -> Vec<(PdfReference, &'a [u8])> {
    let dictionary = objects
        .get(&page.object)
        .and_then(|value| value.model.value.as_dictionary());
    let references = match dictionary.and_then(|value| value.get("Contents")) {
        Some(PdfValue::Reference(reference)) => vec![*reference],
        Some(PdfValue::Array(values)) => values.iter().filter_map(PdfValue::as_reference).collect(),
        _ => Vec::new(),
    };
    references
        .into_iter()
        .filter_map(|reference| {
            objects
                .get(&reference)
                .and_then(|value| {
                    value
                        .decoded_stream
                        .as_deref()
                        .or(value.stream_bytes.as_deref())
                })
                .map(|bytes| (reference, bytes))
        })
        .collect()
}
fn rect_values(value: Option<&PdfValue>) -> Option<[f64; 4]> {
    let PdfValue::Array(values) = value? else {
        return None;
    };
    Some([
        number(values.first()?)?,
        number(values.get(1)?)?,
        number(values.get(2)?)?,
        number(values.get(3)?)?,
    ])
}
fn number(value: &PdfValue) -> Option<f64> {
    match value {
        PdfValue::Integer(value) => Some(*value as f64),
        PdfValue::Real(value) => Some(*value),
        _ => None,
    }
}
fn integer(value: Option<&PdfValue>) -> Option<u64> {
    value
        .and_then(PdfValue::as_integer)
        .and_then(|value| value.try_into().ok())
}

fn fact(
    kind: PdfSemanticEvidenceKind,
    description: impl Into<String>,
    confidence: f64,
    source_blocks: Vec<u64>,
    source_objects: Vec<PdfReference>,
) -> PdfSemanticEvidence {
    PdfSemanticEvidence {
        kind,
        description: description.into(),
        confidence: confidence.clamp(0.0, 1.0),
        source_blocks,
        source_objects,
    }
}
fn block_indexes(blocks: &[PdfSemanticBlock], bbox: BoundingBox) -> Vec<u64> {
    blocks
        .iter()
        .filter(|block| overlap(block.bbox, bbox) > 0.0)
        .map(|block| block.index)
        .collect()
}
fn axis_aligned(line: PdfLineSegment) -> bool {
    (line.start[0] - line.end[0]).abs() <= 0.5 || (line.start[1] - line.end[1]).abs() <= 0.5
}
fn points_bbox(points: impl IntoIterator<Item = [f64; 2]>) -> Option<BoundingBox> {
    let mut points = points.into_iter();
    let first = points.next()?;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first[0], first[0], first[1], first[1]);
    for p in points {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
    }
    Some(BoundingBox {
        x: min_x,
        y: min_y,
        width: (max_x - min_x).max(0.001),
        height: (max_y - min_y).max(0.001),
        unit: CoordinateUnit::Points,
        origin: CoordinateOrigin::BottomLeft,
    })
}
fn union_many(values: impl IntoIterator<Item = BoundingBox>) -> Option<BoundingBox> {
    let mut values = values.into_iter();
    values
        .by_ref()
        .next()
        .map(|first| values.fold(first, union))
}
fn union(a: BoundingBox, b: BoundingBox) -> BoundingBox {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let max_x = right(a).max(right(b));
    let max_y = top(a).max(top(b));
    BoundingBox {
        x,
        y,
        width: max_x - x,
        height: max_y - y,
        unit: CoordinateUnit::Points,
        origin: CoordinateOrigin::BottomLeft,
    }
}
fn overlap(a: BoundingBox, b: BoundingBox) -> f64 {
    let width = (right(a).min(right(b)) - a.x.max(b.x)).max(0.0);
    let height = (top(a).min(top(b)) - a.y.max(b.y)).max(0.0);
    width * height / (a.width * a.height).min(b.width * b.height).max(0.001)
}
fn right(value: BoundingBox) -> f64 {
    value.x + value.width
}
fn top(value: BoundingBox) -> f64 {
    value.y + value.height
}
fn locator(page: &PdfPage, bbox: BoundingBox, confidence: f64) -> SourceLocator {
    let component = LocationComponent::PdfRegion {
        page: IndexPosition::new(page.index, IndexBase::One).expect("one-based page"),
        bbox: Some(bbox),
        rotation_degrees: Some(page.rotation_degrees),
        tokens: None,
    };
    SourceLocator::approximate(
        component,
        LocatorConfidence::new(confidence.clamp(0.0, 1.0)).expect("bounded confidence"),
    )
    .expect("valid semantic locator")
}
