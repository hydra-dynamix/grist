//! Producing-order WordprocessingML stories, tables, fields, and references.

use super::archive::PackageEntry;
use super::formatting::{
    child, children, merge_paragraph_properties, merge_run_formatting, normalized_attributes,
    paragraph_properties, run_formatting, toggle, value,
};
use super::model::*;
use super::numbering::NumberingResolver;
use super::xml_util::{XmlNode, attribute, local_name, parse_xml_part};
use super::{part_locator, xml_locator};
use crate::core::{Diagnostic, IndexPosition, LocationComponent, SourceLocator};

pub(super) struct ContentResult {
    pub body: WordStory,
    pub footnotes: Vec<WordNote>,
    pub endnotes: Vec<WordNote>,
    pub headers: Vec<WordHeaderFooter>,
    pub footers: Vec<WordHeaderFooter>,
    pub node_count: usize,
}

pub(super) fn parse_content(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    main_part: &str,
    styles: &[WordStyle],
    numbering: &WordNumbering,
    diagnostics: &mut Vec<Diagnostic>,
) -> ContentResult {
    let body = parse_part_story(
        entries,
        relationships,
        main_part,
        styles,
        numbering,
        diagnostics,
        StoryRoot::Document,
    )
    .unwrap_or_else(|| empty_story(main_part));
    let footnotes = parse_notes(
        entries,
        relationships,
        main_part,
        styles,
        numbering,
        diagnostics,
        "/footnotes",
    );
    let endnotes = parse_notes(
        entries,
        relationships,
        main_part,
        styles,
        numbering,
        diagnostics,
        "/endnotes",
    );
    let headers = parse_header_footers(
        entries,
        relationships,
        main_part,
        styles,
        numbering,
        diagnostics,
        "/header",
    );
    let footers = parse_header_footers(
        entries,
        relationships,
        main_part,
        styles,
        numbering,
        diagnostics,
        "/footer",
    );
    let node_count = story_node_count(&body)
        + footnotes
            .iter()
            .map(|note| story_node_count(&note.story) + 1)
            .sum::<usize>()
        + endnotes
            .iter()
            .map(|note| story_node_count(&note.story) + 1)
            .sum::<usize>()
        + headers
            .iter()
            .map(|item| story_node_count(&item.story) + 1)
            .sum::<usize>()
        + footers
            .iter()
            .map(|item| story_node_count(&item.story) + 1)
            .sum::<usize>();
    ContentResult {
        body,
        footnotes,
        endnotes,
        headers,
        footers,
        node_count,
    }
}

#[derive(Clone, Copy)]
enum StoryRoot {
    Document,
    HeaderFooter,
}

fn parse_part_story(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    part: &str,
    styles: &[WordStyle],
    numbering: &WordNumbering,
    diagnostics: &mut Vec<Diagnostic>,
    root_kind: StoryRoot,
) -> Option<WordStory> {
    let bytes = entries
        .iter()
        .find(|entry| entry.path == part)
        .and_then(|entry| entry.bytes.as_deref())?;
    let nodes = parse_xml_part(part, bytes, diagnostics).ok()?;
    let root = match root_kind {
        StoryRoot::Document => nodes
            .iter()
            .position(|node| local_name(&node.name) == "body"),
        StoryRoot::HeaderFooter => nodes
            .iter()
            .position(|node| matches!(local_name(&node.name), "hdr" | "ftr")),
    }?;
    let mut parser = StoryParser {
        part,
        nodes: &nodes,
        styles,
        relationships,
        numbering: NumberingResolver::new(numbering),
        paragraph_index: 0,
        table_index: 0,
        section_index: 0,
        current_table: None,
        current_row: None,
        current_column: None,
    };
    Some(parser.story(root))
}

struct StoryParser<'a> {
    part: &'a str,
    nodes: &'a [XmlNode],
    styles: &'a [WordStyle],
    relationships: &'a [WordRelationship],
    numbering: NumberingResolver<'a>,
    paragraph_index: usize,
    table_index: usize,
    section_index: usize,
    current_table: Option<usize>,
    current_row: Option<usize>,
    current_column: Option<usize>,
}

impl StoryParser<'_> {
    fn story(&mut self, root: usize) -> WordStory {
        let blocks = self.blocks(root);
        let sections = children(self.nodes, root, "sectPr")
            .map(|index| self.section(index))
            .collect::<Vec<_>>();
        let visible_text = blocks
            .iter()
            .map(block_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        WordStory {
            part: self.part.to_string(),
            blocks,
            sections,
            visible_text,
            locator: part_locator(self.part).ok(),
        }
    }

    fn blocks(&mut self, parent: usize) -> Vec<WordBlock> {
        self.nodes[parent]
            .children
            .clone()
            .into_iter()
            .filter_map(|index| match local_name(&self.nodes[index].name) {
                "p" => Some(WordBlock::Paragraph(Box::new(self.paragraph(index)))),
                "tbl" => Some(WordBlock::Table(Box::new(self.table(index)))),
                _ => None,
            })
            .collect()
    }

    fn paragraph(&mut self, index: usize) -> WordParagraph {
        self.paragraph_index += 1;
        let paragraph_index = self.paragraph_index;
        let ppr = child(self.nodes, index, "pPr");
        let style_id = ppr
            .and_then(|ppr| child(self.nodes, ppr, "pStyle"))
            .and_then(|item| value(&self.nodes[item]));
        let style = style_id
            .as_ref()
            .and_then(|id| self.styles.iter().find(|style| &style.style_id == id))
            .cloned();
        let direct_properties = paragraph_properties(self.part, self.nodes, ppr);
        let properties = style.as_ref().map_or_else(
            || direct_properties.clone(),
            |style| {
                merge_paragraph_properties(
                    &style.effective_paragraph_properties,
                    &direct_properties,
                )
            },
        );
        let section = ppr
            .and_then(|ppr| child(self.nodes, ppr, "sectPr"))
            .map(|item| self.section(item));
        let locator = self.locator(
            Some(paragraph_index),
            None,
            self.current_table,
            self.current_row,
            self.current_column,
            &self.nodes[index].path,
        );
        let inlines = self.inlines(index, paragraph_index, style.as_ref());
        let text = inline_text(&inlines);
        let fields = collect_fields(&inlines);
        let citations = fields.iter().filter_map(field_citation).collect();
        let cross_references = fields.iter().filter_map(field_reference).collect();
        let numbering_reference = ppr.and_then(|ppr| self.numbering_reference(ppr));
        let numbering = numbering_reference.as_ref().and_then(|reference| {
            self.numbering
                .resolve(reference.num_id, reference.level, reference.locator.clone())
        });
        let heading_level = properties
            .outline_level
            .map(|level| level.saturating_add(1))
            .or_else(|| style.as_ref().and_then(style_heading_level));
        WordParagraph {
            index: paragraph_index,
            text,
            style_id,
            style_name: style.and_then(|style| style.name),
            heading_level,
            direct_properties,
            properties,
            inlines,
            fields,
            citations,
            cross_references,
            section,
            numbering_reference,
            numbering,
            locator,
        }
    }

    fn numbering_reference(&self, ppr: usize) -> Option<WordNumberingReference> {
        let num_pr = child(self.nodes, ppr, "numPr")?;
        let num_id = child(self.nodes, num_pr, "numId")
            .and_then(|item| value(&self.nodes[item]))
            .and_then(|value| value.parse().ok())?;
        let level = child(self.nodes, num_pr, "ilvl")
            .and_then(|item| value(&self.nodes[item]))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        Some(WordNumberingReference {
            num_id,
            level,
            locator: xml_locator(self.part, &self.nodes[num_pr].path),
        })
    }

    fn inlines(
        &self,
        paragraph: usize,
        paragraph_index: usize,
        paragraph_style: Option<&WordStyle>,
    ) -> Vec<WordInline> {
        let mut run_index = 0;
        self.nodes[paragraph]
            .children
            .iter()
            .copied()
            .filter_map(|index| match local_name(&self.nodes[index].name) {
                "r" => Some(WordInline::Run(Box::new(self.run(
                    index,
                    paragraph_index,
                    &mut run_index,
                    paragraph_style,
                )))),
                "hyperlink" => Some(WordInline::Hyperlink(self.hyperlink(
                    index,
                    paragraph_index,
                    &mut run_index,
                    paragraph_style,
                ))),
                "bookmarkStart" => Some(WordInline::Bookmark(self.bookmark(
                    index,
                    paragraph_index,
                    WordBookmarkKind::Start,
                ))),
                "bookmarkEnd" => Some(WordInline::Bookmark(self.bookmark(
                    index,
                    paragraph_index,
                    WordBookmarkKind::End,
                ))),
                "fldSimple" => Some(WordInline::Field(self.simple_field(
                    index,
                    paragraph_index,
                    &mut run_index,
                    paragraph_style,
                ))),
                _ => None,
            })
            .collect()
    }

    fn run(
        &self,
        index: usize,
        paragraph: usize,
        run_index: &mut usize,
        paragraph_style: Option<&WordStyle>,
    ) -> WordRun {
        *run_index += 1;
        let current_run = *run_index;
        let rpr = child(self.nodes, index, "rPr");
        let style_id = rpr
            .and_then(|rpr| child(self.nodes, rpr, "rStyle"))
            .and_then(|item| value(&self.nodes[item]));
        let run_style = style_id
            .as_ref()
            .and_then(|id| self.styles.iter().find(|style| &style.style_id == id));
        let inherited = run_style
            .map(|style| &style.effective_run_formatting)
            .or_else(|| paragraph_style.map(|style| &style.effective_run_formatting))
            .cloned()
            .unwrap_or_default();
        let direct_formatting = run_formatting(self.part, self.nodes, rpr);
        let effective_formatting = merge_run_formatting(&inherited, &direct_formatting);
        let contents = self.run_contents(index, paragraph, current_run);
        WordRun {
            index: current_run,
            text: run_content_text(&contents),
            style_id,
            direct_formatting,
            effective_formatting,
            contents,
            locator: self.locator(
                Some(paragraph),
                Some(current_run),
                self.current_table,
                self.current_row,
                self.current_column,
                &self.nodes[index].path,
            ),
        }
    }

    fn run_contents(&self, run: usize, paragraph: usize, run_index: usize) -> Vec<WordRunContent> {
        self.nodes[run]
            .children
            .iter()
            .copied()
            .filter_map(|index| {
                let node = &self.nodes[index];
                let locator = self.locator(
                    Some(paragraph),
                    Some(run_index),
                    self.current_table,
                    self.current_row,
                    self.current_column,
                    &node.path,
                );
                match local_name(&node.name) {
                    "t" | "delText" => Some(WordRunContent::Text {
                        value: node.text.clone(),
                        preserve_space: attribute(node, "space") == Some("preserve"),
                        locator,
                    }),
                    "tab" | "ptab" => Some(WordRunContent::Tab { locator }),
                    "br" => Some(WordRunContent::Break(WordBreak {
                        break_type: attribute(node, "type").unwrap_or("line").to_string(),
                        clear: attribute(node, "clear").map(str::to_string),
                        locator,
                    })),
                    "cr" => Some(WordRunContent::CarriageReturn { locator }),
                    "softHyphen" => Some(WordRunContent::SoftHyphen { locator }),
                    "noBreakHyphen" => Some(WordRunContent::NoBreakHyphen { locator }),
                    "sym" => Some(WordRunContent::Symbol {
                        font: attribute(node, "font").map(str::to_string),
                        character: attribute(node, "char").map(decode_symbol),
                        locator,
                    }),
                    "footnoteReference" => Some(WordRunContent::FootnoteReference {
                        id: attribute(node, "id").unwrap_or_default().to_string(),
                        locator,
                    }),
                    "endnoteReference" => Some(WordRunContent::EndnoteReference {
                        id: attribute(node, "id").unwrap_or_default().to_string(),
                        locator,
                    }),
                    "instrText" => Some(WordRunContent::FieldInstruction {
                        instruction: node.text.clone(),
                        locator,
                    }),
                    "fldChar" => Some(WordRunContent::FieldCharacter {
                        character_type: attribute(node, "fldCharType")
                            .unwrap_or_default()
                            .to_string(),
                        locked: attribute(node, "fldLock").map(parse_bool),
                        dirty: attribute(node, "dirty").map(parse_bool),
                        locator,
                    }),
                    "lastRenderedPageBreak" => {
                        Some(WordRunContent::LastRenderedPageBreak { locator })
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn hyperlink(
        &self,
        index: usize,
        paragraph: usize,
        run_index: &mut usize,
        paragraph_style: Option<&WordStyle>,
    ) -> WordHyperlink {
        let relationship_id = attribute(&self.nodes[index], "id").map(str::to_string);
        let relationship = relationship_id.as_ref().and_then(|id| {
            self.relationships.iter().find(|relationship| {
                relationship.source_part.as_deref() == Some(self.part) && &relationship.id == id
            })
        });
        let runs = children(self.nodes, index, "r")
            .map(|run| self.run(run, paragraph, run_index, paragraph_style))
            .collect::<Vec<_>>();
        WordHyperlink {
            relationship_id,
            target: relationship.map(|relationship| {
                relationship
                    .resolved_part
                    .clone()
                    .unwrap_or_else(|| relationship.target.clone())
            }),
            anchor: attribute(&self.nodes[index], "anchor").map(str::to_string),
            tooltip: attribute(&self.nodes[index], "tooltip").map(str::to_string),
            history: attribute(&self.nodes[index], "history").map(parse_bool),
            text: runs.iter().map(|run| run.text.as_str()).collect(),
            runs,
            locator: self.locator(
                Some(paragraph),
                None,
                self.current_table,
                self.current_row,
                self.current_column,
                &self.nodes[index].path,
            ),
        }
    }

    fn bookmark(
        &self,
        index: usize,
        paragraph: usize,
        bookmark_kind: WordBookmarkKind,
    ) -> WordBookmark {
        let node = &self.nodes[index];
        let id = attribute(node, "id").unwrap_or_default().to_string();
        WordBookmark {
            id: id.clone(),
            name: attribute(node, "name").map(str::to_string),
            bookmark_kind,
            locator: self.object_locator(
                Some(paragraph),
                None,
                self.current_table,
                self.current_row,
                self.current_column,
                Some(format!("bookmark:{id}")),
                &node.path,
            ),
        }
    }

    fn simple_field(
        &self,
        index: usize,
        paragraph: usize,
        run_index: &mut usize,
        paragraph_style: Option<&WordStyle>,
    ) -> WordField {
        let node = &self.nodes[index];
        let instruction = attribute(node, "instr").unwrap_or_default().to_string();
        let result_runs = children(self.nodes, index, "r")
            .map(|run| self.run(run, paragraph, run_index, paragraph_style))
            .collect::<Vec<_>>();
        let result_text = result_runs.iter().map(|run| run.text.as_str()).collect();
        WordField {
            field_type: field_type(&instruction),
            instruction,
            result_text,
            result_runs,
            locked: attribute(node, "fldLock").map(parse_bool),
            dirty: attribute(node, "dirty").map(parse_bool),
            locator: self.object_locator(
                Some(paragraph),
                None,
                self.current_table,
                self.current_row,
                self.current_column,
                Some(format!("field:{}", self.nodes[index].path)),
                &node.path,
            ),
        }
    }

    fn table(&mut self, index: usize) -> WordTable {
        self.table_index += 1;
        let table_index = self.table_index;
        let previous_table = self.current_table.replace(table_index);
        let table_properties = child(self.nodes, index, "tblPr");
        let style_id = table_properties
            .and_then(|item| child(self.nodes, item, "tblStyle"))
            .and_then(|item| value(&self.nodes[item]));
        let width = table_properties
            .and_then(|item| child(self.nodes, item, "tblW"))
            .and_then(|item| attribute(&self.nodes[item], "w").map(str::to_string));
        let layout = table_properties
            .and_then(|item| child(self.nodes, item, "tblLayout"))
            .and_then(|item| value(&self.nodes[item]));
        let grid_columns = child(self.nodes, index, "tblGrid")
            .map(|grid| {
                children(self.nodes, grid, "gridCol")
                    .map(|column| attribute(&self.nodes[column], "w").map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let row_nodes = children(self.nodes, index, "tr").collect::<Vec<_>>();
        let mut rows = Vec::new();
        for (row_offset, row_node) in row_nodes.into_iter().enumerate() {
            rows.push(self.table_row(row_node, table_index, row_offset + 1));
        }
        resolve_vertical_merges(&mut rows);
        self.current_table = previous_table;
        let text = rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n");
        WordTable {
            index: table_index,
            style_id,
            width,
            layout,
            grid_columns,
            rows,
            text,
            locator: self.locator(
                None,
                None,
                Some(table_index),
                None,
                None,
                &self.nodes[index].path,
            ),
        }
    }

    fn table_row(&mut self, index: usize, table: usize, row: usize) -> WordTableRow {
        let previous_row = self.current_row.replace(row);
        let row_properties = child(self.nodes, index, "trPr");
        let is_header = row_properties
            .and_then(|item| child(self.nodes, item, "tblHeader"))
            .is_some_and(|item| toggle(&self.nodes[item]));
        let cant_split = row_properties
            .and_then(|item| child(self.nodes, item, "cantSplit"))
            .is_some_and(|item| toggle(&self.nodes[item]));
        let height = row_properties
            .and_then(|item| child(self.nodes, item, "trHeight"))
            .and_then(|item| attribute(&self.nodes[item], "val").map(str::to_string));
        let mut cells = Vec::new();
        let mut grid_column = 1;
        for (offset, cell_node) in children(self.nodes, index, "tc").enumerate() {
            let cell = self.table_cell(cell_node, table, row, offset + 1, grid_column);
            grid_column += cell.column_span;
            cells.push(cell);
        }
        self.current_row = previous_row;
        WordTableRow {
            index: row,
            is_header,
            cant_split,
            height,
            cells,
            locator: self.locator(
                None,
                None,
                Some(table),
                Some(row),
                None,
                &self.nodes[index].path,
            ),
        }
    }

    fn table_cell(
        &mut self,
        index: usize,
        table: usize,
        row: usize,
        cell_index: usize,
        grid_column: usize,
    ) -> WordTableCell {
        let previous_column = self.current_column.replace(grid_column);
        let properties = child(self.nodes, index, "tcPr");
        let column_span = properties
            .and_then(|item| child(self.nodes, item, "gridSpan"))
            .and_then(|item| value(&self.nodes[item]))
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        let vertical_merge = properties
            .and_then(|item| child(self.nodes, item, "vMerge"))
            .map(|item| value(&self.nodes[item]).unwrap_or_else(|| "continue".into()));
        let horizontal_merge = properties
            .and_then(|item| child(self.nodes, item, "hMerge"))
            .map(|item| value(&self.nodes[item]).unwrap_or_else(|| "continue".into()));
        let width = properties
            .and_then(|item| child(self.nodes, item, "tcW"))
            .and_then(|item| attribute(&self.nodes[item], "w").map(str::to_string));
        let blocks = self.blocks(index);
        self.current_column = previous_column;
        let text = blocks
            .iter()
            .map(block_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        WordTableCell {
            index: cell_index,
            grid_column,
            column_span,
            row_span: 1,
            vertical_merge,
            horizontal_merge,
            merged_into: None,
            width,
            blocks,
            text,
            locator: self.locator(
                None,
                None,
                Some(table),
                Some(row),
                Some(grid_column),
                &self.nodes[index].path,
            ),
        }
    }

    fn section(&mut self, index: usize) -> WordSection {
        self.section_index += 1;
        let section_index = self.section_index;
        let references = |name: &str| {
            children(self.nodes, index, name)
                .filter_map(|item| {
                    let relationship_id = attribute(&self.nodes[item], "id")?.to_string();
                    let part = self.relationships.iter().find(|relationship| {
                        relationship.source_part.as_deref() == Some(self.part)
                            && relationship.id == relationship_id
                    });
                    Some(WordStoryReference {
                        reference_type: attribute(&self.nodes[item], "type")
                            .unwrap_or("default")
                            .to_string(),
                        relationship_id,
                        part: part.and_then(|relationship| relationship.resolved_part.clone()),
                        locator: xml_locator(self.part, &self.nodes[item].path),
                    })
                })
                .collect::<Vec<_>>()
        };
        let columns = child(self.nodes, index, "cols")
            .map(|item| parse_columns(self.nodes, item))
            .unwrap_or_else(|| WordColumns {
                count: 1,
                ..Default::default()
            });
        WordSection {
            index: section_index,
            break_type: child(self.nodes, index, "type").and_then(|item| value(&self.nodes[item])),
            columns,
            page_size: child(self.nodes, index, "pgSz")
                .map(|item| normalized_attributes(&self.nodes[item]))
                .unwrap_or_default(),
            page_margins: child(self.nodes, index, "pgMar")
                .map(|item| normalized_attributes(&self.nodes[item]))
                .unwrap_or_default(),
            title_page: child(self.nodes, index, "titlePg")
                .is_some_and(|item| toggle(&self.nodes[item])),
            header_references: references("headerReference"),
            footer_references: references("footerReference"),
            locator: xml_locator(self.part, &self.nodes[index].path),
        }
    }

    fn locator(
        &self,
        paragraph: Option<usize>,
        run: Option<usize>,
        table: Option<usize>,
        row: Option<usize>,
        column: Option<usize>,
        path: &str,
    ) -> SourceLocator {
        self.object_locator(paragraph, run, table, row, column, None, path)
    }

    #[allow(clippy::too_many_arguments)]
    fn object_locator(
        &self,
        paragraph: Option<usize>,
        run: Option<usize>,
        table: Option<usize>,
        row: Option<usize>,
        column: Option<usize>,
        object_id: Option<String>,
        path: &str,
    ) -> SourceLocator {
        SourceLocator::exact(LocationComponent::OoxmlPart {
            part: self.part.to_string(),
            paragraph: paragraph.map(one_based),
            run: run.map(one_based),
            table: table.map(one_based),
            row: row.map(one_based),
            column: column.map(one_based),
            object_id,
        })
        .expect("positive WordprocessingML indexes")
        .nested(LocationComponent::XmlPath { path: path.into() })
        .expect("parser-generated absolute XML path")
    }
}

fn one_based(value: usize) -> IndexPosition {
    IndexPosition::one_based(u64::try_from(value).unwrap_or(u64::MAX))
        .expect("WordprocessingML indexes are positive")
}

fn parse_columns(nodes: &[XmlNode], index: usize) -> WordColumns {
    WordColumns {
        count: attribute(&nodes[index], "num")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1),
        spacing: attribute(&nodes[index], "space").map(str::to_string),
        equal_width: attribute(&nodes[index], "equalWidth").map(parse_bool),
        separator: attribute(&nodes[index], "sep").map(parse_bool),
        definitions: children(nodes, index, "col")
            .map(|item| normalized_attributes(&nodes[item]))
            .collect(),
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_notes(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    main_part: &str,
    styles: &[WordStyle],
    numbering: &WordNumbering,
    diagnostics: &mut Vec<Diagnostic>,
    suffix: &str,
) -> Vec<WordNote> {
    let Some(part) = related_parts(relationships, main_part, suffix)
        .into_iter()
        .next()
    else {
        return Vec::new();
    };
    let Some(bytes) = part_bytes(entries, &part) else {
        return Vec::new();
    };
    let Ok(nodes) = parse_xml_part(&part, bytes, diagnostics) else {
        return Vec::new();
    };
    let root_name = suffix.trim_start_matches('/');
    let Some(root) = nodes
        .iter()
        .position(|node| local_name(&node.name) == root_name)
    else {
        return Vec::new();
    };
    nodes[root]
        .children
        .iter()
        .copied()
        .filter(|index| matches!(local_name(&nodes[*index].name), "footnote" | "endnote"))
        .map(|index| {
            let node = &nodes[index];
            let id = attribute(node, "id").unwrap_or_default().to_string();
            let mut parser = StoryParser {
                part: &part,
                nodes: &nodes,
                styles,
                relationships,
                numbering: NumberingResolver::new(numbering),
                paragraph_index: 0,
                table_index: 0,
                section_index: 0,
                current_table: None,
                current_row: None,
                current_column: None,
            };
            WordNote {
                id,
                note_type: attribute(node, "type").map(str::to_string),
                story: parser.story(index),
                locator: xml_locator(&part, &node.path),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn parse_header_footers(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    main_part: &str,
    styles: &[WordStyle],
    numbering: &WordNumbering,
    diagnostics: &mut Vec<Diagnostic>,
    suffix: &str,
) -> Vec<WordHeaderFooter> {
    related_parts(relationships, main_part, suffix)
        .into_iter()
        .filter_map(|part| {
            let story = parse_part_story(
                entries,
                relationships,
                &part,
                styles,
                numbering,
                diagnostics,
                StoryRoot::HeaderFooter,
            )?;
            let relationship_ids = relationships
                .iter()
                .filter(|relationship| relationship.resolved_part.as_deref() == Some(&part))
                .map(|relationship| relationship.id.clone())
                .collect();
            Some(WordHeaderFooter {
                locator: part_locator(&part).expect("validated related part"),
                part,
                relationship_ids,
                story,
            })
        })
        .collect()
}

fn related_parts(relationships: &[WordRelationship], source: &str, suffix: &str) -> Vec<String> {
    let mut parts = relationships
        .iter()
        .filter(|relationship| {
            relationship.source_part.as_deref() == Some(source)
                && relationship
                    .relationship_type
                    .to_ascii_lowercase()
                    .ends_with(suffix)
                && relationship.target_exists == Some(true)
        })
        .filter_map(|relationship| relationship.resolved_part.clone())
        .collect::<Vec<_>>();
    parts.sort();
    parts.dedup();
    parts
}

fn part_bytes<'a>(entries: &'a [PackageEntry], part: &str) -> Option<&'a [u8]> {
    entries
        .iter()
        .find(|entry| entry.path == part)
        .and_then(|entry| entry.bytes.as_deref())
}

fn empty_story(part: &str) -> WordStory {
    WordStory {
        part: part.into(),
        locator: part_locator(part).ok(),
        ..Default::default()
    }
}

fn block_text(block: &WordBlock) -> &str {
    match block {
        WordBlock::Paragraph(paragraph) => &paragraph.text,
        WordBlock::Table(table) => &table.text,
    }
}

fn inline_text(inlines: &[WordInline]) -> String {
    inlines
        .iter()
        .map(|inline| match inline {
            WordInline::Run(run) => run.text.as_str(),
            WordInline::Hyperlink(link) => link.text.as_str(),
            WordInline::Field(field) => field.result_text.as_str(),
            WordInline::Bookmark(_) => "",
        })
        .collect()
}

fn run_content_text(contents: &[WordRunContent]) -> String {
    contents
        .iter()
        .map(|content| match content {
            WordRunContent::Text { value, .. } => value.as_str(),
            WordRunContent::Tab { .. } => "\t",
            WordRunContent::Break(WordBreak { break_type, .. }) if break_type == "page" => "\u{c}",
            WordRunContent::Break(_) | WordRunContent::CarriageReturn { .. } => "\n",
            WordRunContent::SoftHyphen { .. } => "\u{ad}",
            WordRunContent::NoBreakHyphen { .. } => "\u{2011}",
            WordRunContent::Symbol { character, .. } => character.as_deref().unwrap_or(""),
            _ => "",
        })
        .collect()
}

struct ActiveField {
    instruction: String,
    result_text: String,
    locked: Option<bool>,
    dirty: Option<bool>,
    locator: SourceLocator,
    separated: bool,
}

fn collect_fields(inlines: &[WordInline]) -> Vec<WordField> {
    let mut fields = inlines
        .iter()
        .filter_map(|inline| match inline {
            WordInline::Field(field) => Some(field.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut active: Option<ActiveField> = None;
    for content in inline_run_contents(inlines) {
        match content {
            WordRunContent::FieldCharacter {
                character_type,
                locked,
                dirty,
                locator,
            } if character_type == "begin" => {
                active = Some(ActiveField {
                    instruction: String::new(),
                    result_text: String::new(),
                    locked: *locked,
                    dirty: *dirty,
                    locator: locator.clone(),
                    separated: false,
                });
            }
            WordRunContent::FieldCharacter { character_type, .. }
                if character_type == "separate" =>
            {
                if let Some(active) = &mut active {
                    active.separated = true;
                }
            }
            WordRunContent::FieldCharacter { character_type, .. } if character_type == "end" => {
                if let Some(active) = active.take() {
                    fields.push(WordField {
                        field_type: field_type(&active.instruction),
                        instruction: active.instruction.trim().to_string(),
                        result_text: active.result_text,
                        result_runs: Vec::new(),
                        locked: active.locked,
                        dirty: active.dirty,
                        locator: active.locator,
                    });
                }
            }
            WordRunContent::FieldInstruction { instruction, .. } => {
                if let Some(active) = &mut active {
                    active.instruction.push_str(instruction);
                }
            }
            WordRunContent::Text { value, .. } => {
                if let Some(active) = &mut active
                    && active.separated
                {
                    active.result_text.push_str(value);
                }
            }
            _ => {}
        }
    }
    fields
}

fn inline_run_contents(inlines: &[WordInline]) -> Vec<&WordRunContent> {
    let mut output = Vec::new();
    for inline in inlines {
        match inline {
            WordInline::Run(run) => output.extend(run.contents.iter()),
            WordInline::Hyperlink(link) => {
                output.extend(link.runs.iter().flat_map(|run| run.contents.iter()))
            }
            WordInline::Bookmark(_) | WordInline::Field(_) => {}
        }
    }
    output
}

fn field_type(instruction: &str) -> String {
    instruction
        .split_whitespace()
        .next()
        .unwrap_or("UNKNOWN")
        .to_ascii_uppercase()
}

fn field_citation(field: &WordField) -> Option<WordCitation> {
    if field.field_type != "CITATION" && !field.instruction.contains("CSL_CITATION") {
        return None;
    }
    let tags = field
        .instruction
        .split_whitespace()
        .skip(1)
        .take_while(|token| !token.starts_with(char::from(92)))
        .map(|token| token.trim_matches(char::from(34)).to_string())
        .filter(|token| !token.is_empty())
        .collect();
    Some(WordCitation {
        instruction: field.instruction.clone(),
        tags,
        result_text: field.result_text.clone(),
        locator: field.locator.clone(),
    })
}

fn field_reference(field: &WordField) -> Option<WordCrossReference> {
    if !matches!(field.field_type.as_str(), "REF" | "PAGEREF" | "NOTEREF") {
        return None;
    }
    Some(WordCrossReference {
        reference_type: field.field_type.clone(),
        target: field.instruction.split_whitespace().nth(1)?.to_string(),
        result_text: field.result_text.clone(),
        locator: field.locator.clone(),
    })
}

fn resolve_vertical_merges(rows: &mut [WordTableRow]) {
    let mut origins = std::collections::BTreeMap::<usize, (usize, usize)>::new();
    for row_index in 0..rows.len() {
        let mut horizontal_origin = None;
        for cell_index in 0..rows[row_index].cells.len() {
            let grid_column = rows[row_index].cells[cell_index].grid_column;
            let column_span = rows[row_index].cells[cell_index].column_span;
            match rows[row_index].cells[cell_index].vertical_merge.as_deref() {
                Some("restart") => {
                    for column in grid_column..grid_column + column_span {
                        origins.insert(column, (row_index, cell_index));
                    }
                }
                Some("continue") => {
                    if let Some(&(origin_row, origin_cell)) = origins.get(&grid_column) {
                        rows[row_index].cells[cell_index].merged_into =
                            Some((origin_row + 1, origin_cell + 1));
                        rows[origin_row].cells[origin_cell].row_span += 1;
                    }
                }
                _ => {
                    for column in grid_column..grid_column + column_span {
                        origins.remove(&column);
                    }
                }
            }
            match rows[row_index].cells[cell_index]
                .horizontal_merge
                .as_deref()
            {
                Some("restart") => horizontal_origin = Some(cell_index),
                Some("continue") => {
                    if let Some(origin) = horizontal_origin {
                        rows[row_index].cells[cell_index].merged_into =
                            Some((row_index + 1, origin + 1));
                        rows[row_index].cells[origin].column_span += column_span;
                    }
                }
                _ => horizontal_origin = None,
            }
        }
    }
}

fn style_heading_level(style: &WordStyle) -> Option<u8> {
    style
        .effective_paragraph_properties
        .outline_level
        .map(|level| level.saturating_add(1))
        .or_else(|| {
            let name = style.name.as_deref()?.to_ascii_lowercase();
            name.strip_prefix("heading")
                .and_then(|suffix| suffix.trim().parse().ok())
        })
}

fn parse_bool(value: &str) -> bool {
    !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "off")
}

fn decode_symbol(value: &str) -> String {
    u32::from_str_radix(value, 16)
        .ok()
        .and_then(char::from_u32)
        .map(String::from)
        .unwrap_or_else(|| value.to_string())
}

fn story_node_count(story: &WordStory) -> usize {
    story.sections.len()
        + story
            .blocks
            .iter()
            .map(|block| match block {
                WordBlock::Paragraph(paragraph) => {
                    1 + paragraph.inlines.len() + paragraph.fields.len()
                }
                WordBlock::Table(table) => {
                    1 + table
                        .rows
                        .iter()
                        .map(|row| {
                            1 + row
                                .cells
                                .iter()
                                .map(|cell| {
                                    1 + story_node_count(&WordStory {
                                        blocks: cell.blocks.clone(),
                                        ..Default::default()
                                    })
                                })
                                .sum::<usize>()
                        })
                        .sum::<usize>()
                }
            })
            .sum::<usize>()
}
