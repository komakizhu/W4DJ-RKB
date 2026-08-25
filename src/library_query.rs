//! Whitelisted, parameterized queries for the library dashboard.

use crate::library_catalog::{CatalogTrack, LibraryError};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryField {
    Title,
    Artists,
    Album,
    FileName,
    LocalStatus,
    Format,
    Bitrate,
    FileSize,
    Duration,
    NeteaseGenre,
    EssentiaGenre,
    Bpm,
    MusicalKey,
    Loudness,
    Energy,
    Danceability,
    DiscogsMoodTheme,
    DiscogsApproachability,
    DiscogsInstrumentation,
    DiscogsTimbre,
    DiscogsDanceability,
    CoverAvailable,
    Lyrics,
    UpdatedAt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryOperator {
    Is,
    IsNot,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    Between,
    IsEmpty,
    IsNotEmpty,
    IsTrue,
    IsFalse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterLogic {
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFilter {
    pub field: LibraryField,
    pub operator: LibraryOperator,
    pub value: Option<String>,
    pub second_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySort {
    pub field: LibraryField,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryQuery {
    pub text: String,
    pub filters: Vec<LibraryFilter>,
    pub filter_logic: FilterLogic,
    pub sorts: Vec<LibrarySort>,
    pub limit: u32,
    pub offset: u32,
}

impl Default for LibraryQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            filters: Vec::new(),
            filter_logic: FilterLogic::And,
            sorts: Vec::new(),
            limit: 100,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage {
    pub items: Vec<CatalogTrack>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

pub(crate) fn compile_query(query: &LibraryQuery) -> Result<CompiledQuery, LibraryError> {
    let limit = query.limit.clamp(1, 500);
    let offset = query.offset;
    let mut predicates = Vec::new();
    let mut values = Vec::new();

    let text = query.text.trim();
    if !text.is_empty() {
        predicates.push(
            "(t.title LIKE ? ESCAPE '\\' OR t.artists LIKE ? ESCAPE '\\' OR t.album LIKE ? ESCAPE '\\' OR t.aliases_json LIKE ? ESCAPE '\\' OR t.netease_genre LIKE ? ESCAPE '\\' OR t.essentia_genre LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM local_files lf WHERE lf.track_key=t.track_key AND lf.path LIKE ? ESCAPE '\\'))".to_string(),
        );
        let pattern = format!("%{}%", escape_like(text));
        values.extend((0..7).map(|_| Value::Text(pattern.clone())));
    }

    let joiner = match query.filter_logic {
        FilterLogic::And => " AND ",
        FilterLogic::Or => " OR ",
    };
    let filter_predicates = query
        .filters
        .iter()
        .map(|filter| compile_filter(filter, &mut values))
        .collect::<Result<Vec<_>, _>>()?;
    if !filter_predicates.is_empty() {
        predicates.push(format!("({})", filter_predicates.join(joiner)));
    }

    let where_sql = if predicates.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", predicates.join(" AND "))
    };

    let order_sql = if query.sorts.is_empty() {
        " ORDER BY t.title COLLATE NOCASE ASC, t.track_key ASC".to_string()
    } else {
        let terms = query
            .sorts
            .iter()
            .map(|sort| {
                let field = field_expression(&sort.field)?;
                Ok(format!("{} {}", field, direction_sql(&sort.direction)))
            })
            .collect::<Result<Vec<_>, LibraryError>>()?;
        format!(" ORDER BY {}, t.track_key ASC", terms.join(", "))
    };

    Ok(CompiledQuery {
        where_sql,
        order_sql,
        values,
        limit,
        offset,
    })
}

pub(crate) struct CompiledQuery {
    pub where_sql: String,
    pub order_sql: String,
    pub values: Vec<Value>,
    pub limit: u32,
    pub offset: u32,
}

fn compile_filter(filter: &LibraryFilter, values: &mut Vec<Value>) -> Result<String, LibraryError> {
    let confidence_filter = matches!(
        filter.operator,
        LibraryOperator::GreaterThan
            | LibraryOperator::GreaterOrEqual
            | LibraryOperator::LessThan
            | LibraryOperator::LessOrEqual
            | LibraryOperator::Between
    );
    let expression = if confidence_filter {
        discogs_confidence_expression(&filter.field).unwrap_or(field_expression(&filter.field)?)
    } else {
        field_expression(&filter.field)?
    };
    match filter.operator {
        LibraryOperator::IsEmpty => Ok(format!("({expression} IS NULL OR {expression} = '')")),
        LibraryOperator::IsNotEmpty => {
            Ok(format!("({expression} IS NOT NULL AND {expression} <> '')"))
        }
        LibraryOperator::IsTrue => Ok(format!("{expression} = 1")),
        LibraryOperator::IsFalse => Ok(format!("({expression} = 0 OR {expression} IS NULL)")),
        LibraryOperator::Is
        | LibraryOperator::IsNot
        | LibraryOperator::Contains
        | LibraryOperator::NotContains
        | LibraryOperator::StartsWith
        | LibraryOperator::EndsWith
        | LibraryOperator::GreaterThan
        | LibraryOperator::GreaterOrEqual
        | LibraryOperator::LessThan
        | LibraryOperator::LessOrEqual => {
            let raw = filter
                .value
                .as_deref()
                .ok_or_else(|| LibraryError::Invalid("筛选条件缺少值".to_string()))?;
            let numeric = is_numeric_field(&filter.field)
                || (confidence_filter && discogs_confidence_expression(&filter.field).is_some());
            let value = if numeric {
                Value::Real(parse_number(raw)?)
            } else {
                Value::Text(raw.to_string())
            };
            let operator = match filter.operator {
                LibraryOperator::Is => "=",
                LibraryOperator::IsNot => "<>",
                LibraryOperator::GreaterThan => ">",
                LibraryOperator::GreaterOrEqual => ">=",
                LibraryOperator::LessThan => "<",
                LibraryOperator::LessOrEqual => "<=",
                LibraryOperator::Contains
                | LibraryOperator::NotContains
                | LibraryOperator::StartsWith
                | LibraryOperator::EndsWith => "LIKE",
                _ => unreachable!(),
            };
            let value = match filter.operator {
                LibraryOperator::Contains | LibraryOperator::NotContains => {
                    Value::Text(format!("%{}%", escape_like(raw)))
                }
                LibraryOperator::StartsWith => Value::Text(format!("{}%", escape_like(raw))),
                LibraryOperator::EndsWith => Value::Text(format!("%{}", escape_like(raw))),
                _ => value,
            };
            values.push(value);
            let comparison = if matches!(filter.operator, LibraryOperator::NotContains) {
                "NOT LIKE"
            } else {
                operator
            };
            if matches!(
                filter.operator,
                LibraryOperator::Contains
                    | LibraryOperator::NotContains
                    | LibraryOperator::StartsWith
                    | LibraryOperator::EndsWith
            ) {
                Ok(format!("{expression} {comparison} ? ESCAPE '\\'"))
            } else {
                Ok(format!("{expression} {comparison} ?"))
            }
        }
        LibraryOperator::Between => {
            if !is_numeric_field(&filter.field)
                && discogs_confidence_expression(&filter.field).is_none()
            {
                return Err(LibraryError::Invalid(
                    "between 只适用于数值字段".to_string(),
                ));
            }
            let first = filter
                .value
                .as_deref()
                .ok_or_else(|| LibraryError::Invalid("between 缺少下界".to_string()))?;
            let second = filter
                .second_value
                .as_deref()
                .ok_or_else(|| LibraryError::Invalid("between 缺少上界".to_string()))?;
            values.push(Value::Real(parse_number(first)?));
            values.push(Value::Real(parse_number(second)?));
            Ok(format!("{expression} BETWEEN ? AND ?"))
        }
    }
}

fn field_expression(field: &LibraryField) -> Result<&'static str, LibraryError> {
    Ok(match field {
        LibraryField::Title => "t.title",
        LibraryField::Artists => "t.artists",
        LibraryField::Album => "t.album",
        LibraryField::FileName => {
            "(SELECT group_concat(lf.path, ' ') FROM local_files lf WHERE lf.track_key=t.track_key)"
        }
        LibraryField::LocalStatus => "t.local_status",
        LibraryField::Format => "t.effective_format",
        LibraryField::Bitrate => "t.effective_bitrate_bps",
        LibraryField::FileSize => "t.effective_size_bytes",
        LibraryField::Duration => "t.effective_duration_seconds",
        LibraryField::NeteaseGenre => "t.netease_genre",
        LibraryField::EssentiaGenre => "t.essentia_genre",
        LibraryField::Bpm => "t.bpm",
        LibraryField::MusicalKey => "t.musical_key",
        LibraryField::Loudness => "t.integrated_loudness_lufs",
        LibraryField::Energy => "t.energy",
        LibraryField::Danceability => "t.danceability",
        LibraryField::DiscogsMoodTheme => "t.discogs_mood_theme_json",
        LibraryField::DiscogsApproachability => "t.discogs_approachability_json",
        LibraryField::DiscogsInstrumentation => "t.discogs_instrumentation_json",
        LibraryField::DiscogsTimbre => "t.discogs_timbre_json",
        LibraryField::DiscogsDanceability => "t.discogs_danceability_json",
        LibraryField::CoverAvailable => "t.cover_available",
        LibraryField::Lyrics => "t.lyric_plain_text",
        LibraryField::UpdatedAt => "t.updated_at_ms",
    })
}

fn is_numeric_field(field: &LibraryField) -> bool {
    matches!(
        field,
        LibraryField::Bitrate
            | LibraryField::FileSize
            | LibraryField::Duration
            | LibraryField::Bpm
            | LibraryField::Loudness
            | LibraryField::Energy
            | LibraryField::Danceability
            | LibraryField::UpdatedAt
    )
}

fn discogs_confidence_expression(field: &LibraryField) -> Option<&'static str> {
    Some(match field {
        // Multi-label heads are sorted by confidence before persistence, so
        // the first label is the strongest displayed confidence.
        LibraryField::DiscogsMoodTheme => {
            "json_extract(t.discogs_mood_theme_json, '$.labels[0].confidence')"
        }
        LibraryField::DiscogsInstrumentation => {
            "json_extract(t.discogs_instrumentation_json, '$.labels[0].confidence')"
        }
        LibraryField::DiscogsApproachability => {
            "json_extract(t.discogs_approachability_json, '$.selectedConfidence')"
        }
        LibraryField::DiscogsTimbre => {
            "json_extract(t.discogs_timbre_json, '$.selectedConfidence')"
        }
        LibraryField::DiscogsDanceability => {
            "json_extract(t.discogs_danceability_json, '$.selectedConfidence')"
        }
        _ => return None,
    })
}

fn parse_number(value: &str) -> Result<f64, LibraryError> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or_else(|| LibraryError::Invalid(format!("不是有效数值：{value}")))
}

fn direction_sql(direction: &SortDirection) -> &'static str {
    match direction {
        SortDirection::Asc => "ASC",
        SortDirection::Desc => "DESC",
    }
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::{
        FilterLogic, LibraryField, LibraryFilter, LibraryOperator, LibraryQuery, LibrarySort,
        SortDirection, compile_query,
    };

    #[test]
    fn compiles_text_and_numeric_filters_without_raw_sql_values() {
        let query = LibraryQuery {
            text: "Tyla".to_string(),
            filters: vec![LibraryFilter {
                field: LibraryField::Bitrate,
                operator: LibraryOperator::GreaterOrEqual,
                value: Some("320000".to_string()),
                second_value: None,
            }],
            filter_logic: FilterLogic::And,
            sorts: vec![LibrarySort {
                field: LibraryField::Album,
                direction: SortDirection::Asc,
            }],
            limit: 999,
            offset: 20,
        };
        let compiled = compile_query(&query).unwrap();
        assert!(compiled.where_sql.contains("t.effective_bitrate_bps >= ?"));
        assert!(compiled.where_sql.contains("t.aliases_json LIKE ?"));
        assert_eq!(compiled.values.len(), 8);
        assert_eq!(compiled.limit, 500);
        assert!(compiled.order_sql.contains("t.album ASC"));
    }

    #[test]
    fn rejects_non_numeric_between() {
        let query = LibraryQuery {
            filters: vec![LibraryFilter {
                field: LibraryField::Artists,
                operator: LibraryOperator::Between,
                value: Some("a".to_string()),
                second_value: Some("z".to_string()),
            }],
            ..LibraryQuery::default()
        };
        assert!(compile_query(&query).is_err());
    }

    #[test]
    fn supports_discogs_class_text_and_confidence_filters() {
        let query = LibraryQuery {
            filters: vec![
                LibraryFilter {
                    field: LibraryField::DiscogsDanceability,
                    operator: LibraryOperator::Is,
                    value: Some("danceable".into()),
                    second_value: None,
                },
                LibraryFilter {
                    field: LibraryField::DiscogsDanceability,
                    operator: LibraryOperator::GreaterOrEqual,
                    value: Some("0.8".into()),
                    second_value: None,
                },
            ],
            ..LibraryQuery::default()
        };
        let compiled = compile_query(&query).unwrap();
        assert!(
            compiled
                .where_sql
                .contains("t.discogs_danceability_json = ?")
        );
        assert!(
            compiled
                .where_sql
                .contains("json_extract(t.discogs_danceability_json, '$.selectedConfidence') >= ?")
        );
        assert_eq!(compiled.values.len(), 2);
    }
}
