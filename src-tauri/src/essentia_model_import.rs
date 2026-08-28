use flate2::read::DeflateDecoder;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_IMPORT_FILES: usize = 16;
pub const MAX_MODEL_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_MODEL_BATCH_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_ARCHIVE_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 32;
const MAX_MALFORMED_DATA_OFFSET: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelImportIssue {
    pub file_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelImportReport {
    pub installed_ids: Vec<String>,
    pub issues: Vec<ModelImportIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchiveEntry {
    name: String,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct KnownModelIdentity {
    id: &'static str,
    path_markers: &'static [&'static str],
    archive: bool,
    expected_output_units: Option<u64>,
    expected_output_name: Option<&'static str>,
    expected_input_width: Option<u64>,
    expected_input_shape: Option<&'static [u64]>,
}

const KNOWN_MODELS: &[KnownModelIdentity] = &[
    KnownModelIdentity {
        id: "musicnn_embedding",
        path_markers: &["msd-musicnn-1-tfjs"],
        archive: true,
        expected_output_units: None,
        expected_output_name: None,
        expected_input_width: None,
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "mood_aggressive",
        path_markers: &["mood_aggressive-msd-musicnn-1-tfjs", "mood_aggressive"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: None,
        expected_input_width: None,
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "mood_happy",
        path_markers: &["mood_happy-msd-musicnn-1-tfjs", "mood_happy"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: None,
        expected_input_width: None,
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "mood_relaxed",
        path_markers: &["mood_relaxed-msd-musicnn-1-tfjs", "mood_relaxed"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: None,
        expected_input_width: None,
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "mood_party",
        path_markers: &["mood_party-msd-musicnn-1-tfjs", "mood_party"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: None,
        expected_input_width: None,
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "mood_sad",
        path_markers: &["mood_sad-msd-musicnn-1-tfjs", "mood_sad"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: None,
        expected_input_width: None,
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "voice_instrumental",
        path_markers: &[
            "voice_instrumental-msd-musicnn-1-tfjs",
            "voice_instrumental",
        ],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: None,
        expected_input_width: None,
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "emomusic",
        path_markers: &[
            "emomusic-msd-musicnn-2-tfjs",
            "emomusic-msd-musicnn-2",
            "emomusic-musicnn-msd-1-tfjs",
            "emomusic",
        ],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: Some("model/Identity"),
        expected_input_width: Some(200),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "muse",
        path_markers: &[
            "muse-msd-musicnn-2-tfjs",
            "muse-msd-musicnn-2",
            "muse-musicnn-msd-1-tfjs",
            "muse",
        ],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: Some("model/Identity"),
        expected_input_width: Some(200),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "mirex",
        path_markers: &["moods_mirex-msd-musicnn-1-tfjs", "moods_mirex-msd-musicnn-1", "mirex"],
        archive: false,
        expected_output_units: Some(5),
        expected_output_name: Some("PartitionedCall"),
        expected_input_width: Some(200),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "discogs_effnet",
        path_markers: &["discogs-effnet-bs64-1", "discogs-effnet", "discogs_effnet"],
        archive: false,
        expected_output_units: Some(1280),
        expected_output_name: Some("discogs_embedding"),
        expected_input_width: None,
        expected_input_shape: Some(&[64, 128, 96]),
    },
    KnownModelIdentity {
        id: "discogs_effnet_embedding",
        path_markers: &["discogs-effnet-bs64-1", "discogs_effnet_embedding"],
        archive: false,
        expected_output_units: Some(1280),
        expected_output_name: Some("discogs_embedding"),
        expected_input_width: None,
        expected_input_shape: Some(&[64, 128, 96]),
    },
    KnownModelIdentity {
        id: "genre_discogs400",
        path_markers: &["genre_discogs400-discogs-effnet-1", "genre_discogs400"],
        archive: false,
        expected_output_units: Some(400),
        expected_output_name: Some("discogs_genre"),
        expected_input_width: Some(1280),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "discogs_mood_theme",
        path_markers: &["mtg_jamendo_moodtheme-discogs-effnet-1", "discogs_mood_theme"],
        archive: false,
        expected_output_units: Some(56),
        expected_output_name: Some("model/Sigmoid"),
        expected_input_width: Some(1280),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "discogs_approachability",
        path_markers: &["approachability_2c-discogs-effnet-1", "discogs_approachability"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: Some("model/Softmax"),
        expected_input_width: Some(1280),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "discogs_instrumentation",
        path_markers: &["mtg_jamendo_instrument-discogs-effnet-1", "discogs_instrumentation"],
        archive: false,
        expected_output_units: Some(40),
        expected_output_name: Some("model/Sigmoid"),
        expected_input_width: Some(1280),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "discogs_timbre",
        path_markers: &["timbre-discogs-effnet-1", "discogs_timbre"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: Some("model/Softmax"),
        expected_input_width: Some(1280),
        expected_input_shape: None,
    },
    KnownModelIdentity {
        id: "discogs_danceability",
        path_markers: &["danceability-discogs-effnet-1", "discogs_danceability"],
        archive: false,
        expected_output_units: Some(2),
        expected_output_name: Some("model/Softmax"),
        expected_input_width: Some(1280),
        expected_input_shape: None,
    },
];

pub fn known_import_model_ids() -> &'static [&'static str] {
    const IDS: &[&str] = &[
        "musicnn_embedding",
        "mood_aggressive",
        "mood_happy",
        "mood_relaxed",
        "mood_party",
        "mood_sad",
        "voice_instrumental",
        "emomusic",
        "muse",
        "mirex",
        "discogs_effnet",
        "discogs_effnet_embedding",
        "genre_discogs400",
        "discogs_mood_theme",
        "discogs_approachability",
        "discogs_instrumentation",
        "discogs_timbre",
        "discogs_danceability",
    ];
    IDS
}

#[derive(Debug)]
struct InputFile {
    path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct ModelArtifacts {
    id: &'static str,
    model_json: Vec<u8>,
    weights: Vec<u8>,
}

pub fn import_model_paths(
    paths: &[PathBuf],
    models_path: &Path,
) -> Result<ModelImportReport, String> {
    if paths.is_empty() {
        return Err("未选择模型文件".to_string());
    }
    if paths.len() > MAX_IMPORT_FILES {
        return Err(format!("一次最多导入 {MAX_IMPORT_FILES} 个模型文件"));
    }

    let mut total_bytes = 0u64;
    let mut inputs = Vec::with_capacity(paths.len());
    let mut issues = Vec::new();
    for path in paths {
        let file_name = display_file_name(path);
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        if !matches!(extension.as_deref(), Some("zip" | "json" | "bin")) {
            issues.push(ModelImportIssue {
                file_name,
                reason: "只支持 ZIP、JSON 或 BIN 模型文件".to_string(),
            });
            continue;
        }
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            format!("读取模型文件失败：{file_name}（{error}）")
        })?;
        if !metadata.file_type().is_file() {
            issues.push(ModelImportIssue {
                file_name,
                reason: "模型路径不是普通文件".to_string(),
            });
            continue;
        }
        if metadata.len() > MAX_MODEL_FILE_BYTES {
            return Err(format!("模型文件过大：{MAX_MODEL_FILE_BYTES} 字节以内"));
        }
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| "模型文件总大小超出限制".to_string())?;
        if total_bytes > MAX_MODEL_BATCH_BYTES {
            return Err(format!("本批模型文件总大小超过 {MAX_MODEL_BATCH_BYTES} 字节"));
        }
        let bytes = fs::read(path).map_err(|error| {
            format!("读取模型文件失败：{file_name}（{error}）")
        })?;
        inputs.push(InputFile {
            path: path.clone(),
            bytes,
        });
    }

    fs::create_dir_all(models_path).map_err(|error| format!("创建 Essentia 模型目录失败：{error}"))?;
    let conflicting_ids = conflicting_input_model_ids(&inputs);
    let mut installed_ids = Vec::new();
    let mut consumed = HashSet::new();
    for (index, input) in inputs.iter().enumerate() {
        if extension_is(&input.path, "zip") {
            if !input
                .path
                .to_string_lossy()
                .replace('\\', "/")
                .contains("msd-musicnn-1-tfjs")
            {
                issues.push(ModelImportIssue {
                    file_name: display_file_name(&input.path),
                    reason: "无法确认这是官方 MusiCNN 模型包".to_string(),
                });
                consumed.insert(index);
                continue;
            }
            if conflicting_ids.contains("musicnn_embedding") {
                issues.push(ModelImportIssue {
                    file_name: display_file_name(&input.path),
                    reason: "同一模型包含多个冲突版本，本批未安装".to_string(),
                });
                consumed.insert(index);
                continue;
            }
            let result = extract_official_musicnn_pair(&input.bytes)
                .and_then(|(model_json, weights)| {
                    let identity = KNOWN_MODELS
                        .iter()
                        .find(|identity| identity.id == "musicnn_embedding")
                        .expect("embedding identity is defined");
                    validate_model_json(&model_json, *identity, true)?;
                    Ok(ModelArtifacts {
                        id: identity.id,
                        model_json,
                        weights,
                    })
                });
            match result {
                Ok(artifacts) => match install_artifacts(models_path, artifacts) {
                    Ok(()) => installed_ids.push("musicnn_embedding".to_string()),
                    Err(error) => issues.push(ModelImportIssue {
                        file_name: display_file_name(&input.path),
                        reason: error,
                    }),
                },
                Err(error) => issues.push(ModelImportIssue {
                    file_name: display_file_name(&input.path),
                    reason: error,
                }),
            }
            consumed.insert(index);
        }
    }

    for (index, input) in inputs.iter().enumerate() {
        if consumed.contains(&index) || !extension_is(&input.path, "json") {
            continue;
        }
        let identity = match identify_model(&input.path) {
            Ok(identity) => identity,
            Err(error) => {
                issues.push(ModelImportIssue {
                    file_name: display_file_name(&input.path),
                    reason: error,
                });
                consumed.insert(index);
                continue;
            }
        };
        if conflicting_ids.contains(identity.id) {
            issues.push(ModelImportIssue {
                file_name: display_file_name(&input.path),
                reason: "同一模型包含多个冲突版本，本批未安装".to_string(),
            });
            consumed.insert(index);
            continue;
        }
        let artifacts = match build_loose_artifacts(input, &inputs, identity) {
            Ok(artifacts) => artifacts,
            Err(error) => {
                issues.push(ModelImportIssue {
                    file_name: display_file_name(&input.path),
                    reason: error,
                });
                consumed.insert(index);
                continue;
            }
        };
        match install_artifacts(models_path, artifacts) {
            Ok(()) => {
                installed_ids.push(identity.id.to_string());
                for (weight_index, weight_input) in inputs.iter().enumerate() {
                    if extension_is(&weight_input.path, "bin")
                        && weight_input.path.parent() == input.path.parent()
                    {
                        consumed.insert(weight_index);
                    }
                }
            }
            Err(error) => issues.push(ModelImportIssue {
                file_name: display_file_name(&input.path),
                reason: error,
            }),
        }
        consumed.insert(index);
    }

    for (index, input) in inputs.iter().enumerate() {
        if consumed.contains(&index) || extension_is(&input.path, "json") {
            continue;
        }
        if extension_is(&input.path, "bin") {
            issues.push(ModelImportIssue {
                file_name: display_file_name(&input.path),
                reason: "缺少对应的 model.json".to_string(),
            });
        }
    }

    installed_ids.sort();
    installed_ids.dedup();
    Ok(ModelImportReport {
        installed_ids,
        issues,
    })
}

fn conflicting_input_model_ids(inputs: &[InputFile]) -> HashSet<&'static str> {
    let mut counts = HashMap::new();
    for input in inputs {
        let id = if extension_is(&input.path, "zip")
            && normalized_path_contains(&input.path, "msd-musicnn-1-tfjs")
        {
            Some("musicnn_embedding")
        } else if extension_is(&input.path, "json") {
            identify_model(&input.path).ok().map(|identity| identity.id).or_else(|| {
                normalized_path_contains(&input.path, "msd-musicnn-1-tfjs")
                    .then_some("musicnn_embedding")
            })
        } else {
            None
        };
        if let Some(id) = id {
            *counts.entry(id).or_insert(0usize) += 1;
        }
    }
    counts
        .into_iter()
        .filter_map(|(id, count)| (count > 1).then_some(id))
        .collect()
}

fn normalized_path_contains(path: &Path, marker: &str) -> bool {
    path.to_string_lossy().replace('\\', "/").contains(marker)
}

fn build_loose_artifacts(
    model_file: &InputFile,
    inputs: &[InputFile],
    identity: KnownModelIdentity,
) -> Result<ModelArtifacts, String> {
    let manifest_paths = validate_model_json(&model_file.bytes, identity, false)?;
    let mut weights = Vec::new();
    let parent = model_file.path.parent().unwrap_or_else(|| Path::new("."));
    for manifest_path in manifest_paths {
        let expected_path = parent.join(&manifest_path);
        let exact = inputs.iter().find(|input| input.path == expected_path);
        let matching = exact.or_else(|| {
            let base = Path::new(&manifest_path).file_name()?;
            let candidates = inputs.iter().filter(|input| {
                input.path.file_name() == Some(base) && extension_is(&input.path, "bin")
            });
            let mut found = None;
            for candidate in candidates {
                if found.is_some() {
                    return None;
                }
                found = Some(candidate);
            }
            found
        });
        let Some(weight_file) = matching else {
            return Err(format!("缺少权重文件：{}", display_relative_name(&manifest_path)));
        };
        weights.extend_from_slice(&weight_file.bytes);
    }
    if weights.is_empty() {
        return Err("模型权重为空".to_string());
    }
    Ok(ModelArtifacts {
        id: identity.id,
        model_json: model_file.bytes.clone(),
        weights,
    })
}

fn identify_model(path: &Path) -> Result<KnownModelIdentity, String> {
    let mut matches = Vec::new();
    for identity in KNOWN_MODELS.iter().copied().filter(|identity| !identity.archive) {
        if identity
            .path_markers
            .iter()
            .any(|marker| path_matches_marker(path, marker))
        {
            matches.push(identity);
        }
    }
    if matches.len() == 1 {
        return Ok(matches[0]);
    }
    Err(if matches.is_empty() {
        "无法识别 Essentia 模型；请使用官方模型目录中的文件名和文件夹".to_string()
    } else {
        "模型身份不唯一；请不要混用或重命名官方模型文件".to_string()
    })
}

fn path_matches_marker(path: &Path, marker: &str) -> bool {
    path.components().any(|component| {
        let component = component.as_os_str().to_string_lossy();
        component == marker
            || Path::new(component.as_ref())
                .file_stem()
                .is_some_and(|stem| stem == marker)
    })
}

fn validate_model_json(
    bytes: &[u8],
    identity: KnownModelIdentity,
    archive: bool,
) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| format!("model.json 格式无效：{error}"))?;
    if value.get("modelTopology").is_none() {
        return Err("model.json 缺少 modelTopology".to_string());
    }
    let manifests = value
        .get("weightsManifest")
        .and_then(Value::as_array)
        .filter(|manifests| !manifests.is_empty())
        .ok_or_else(|| "model.json 缺少 weightsManifest".to_string())?;
    let mut paths = Vec::new();
    for manifest in manifests {
        let manifest_paths = manifest
            .get("paths")
            .and_then(Value::as_array)
            .ok_or_else(|| "weightsManifest 缺少 paths".to_string())?;
        for path in manifest_paths {
            let path = path
                .as_str()
                .ok_or_else(|| "weightsManifest 包含无效权重路径".to_string())?;
            validate_relative_path(path)?;
            paths.push(path.to_string());
        }
    }
    if paths.is_empty() {
        return Err("weightsManifest 为空".to_string());
    }
    if !archive
        && let Some(expected) = identity.expected_output_units
        && model_output_units(&value, identity.expected_output_name) != Some(expected)
    {
        return Err(format!("模型输出维度不是预期的 {expected}"));
    }
    if !archive
        && identity.expected_output_units.is_some()
        && identity.expected_input_width.is_none()
        && identity.expected_input_shape.is_none()
        && model_input_units(&value) != Some(200)
    {
        return Err("分类模型输入维度不是预期的 200".to_string());
    }
    if !archive
        && let Some(expected_width) = identity.expected_input_width
        && model_input_units(&value) != Some(expected_width)
    {
        return Err(format!("模型输入维度不是预期的 {expected_width}"));
    }
    if !archive
        && let Some(expected_shape) = identity.expected_input_shape
        && model_input_shape(&value).as_deref() != Some(expected_shape)
    {
        return Err(format!("模型输入形状不是预期的 {expected_shape:?}"));
    }
    if !archive
        && let Some(expected_name) = identity.expected_output_name
        && !model_output_node_matches(&value, identity.id, expected_name)
    {
        return Err(format!("模型缺少预期输出节点：{expected_name}"));
    }
    if identity.id == "musicnn_embedding" {
        validate_musicnn_topology(&value)?;
    }
    Ok(paths)
}

fn model_input_units(value: &Value) -> Option<u64> {
    if let Some(width) = model_input_shape(value).and_then(|shape| shape.last().copied())
        && width > 0
    {
        return Some(width);
    }
    let manifest_units = value
        .get("weightsManifest")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|manifest| manifest.get("weights").and_then(Value::as_array))
        .flatten()
        .find_map(|weight| {
            let name = weight.get("name")?.as_str()?;
            if !name.contains("kernel") {
                return None;
            }
            let width = weight.get("shape")?.as_array()?.first()?.as_u64()?;
            (width == 200 || width == 1280).then_some(width)
        });
    if manifest_units.is_some() {
        return manifest_units;
    }
    manifest_units
}

fn model_input_shape(value: &Value) -> Option<Vec<u64>> {
    if let Some(nodes) = value.pointer("/modelTopology/node").and_then(Value::as_array)
        && let Some(placeholder) = nodes
            .iter()
            .find(|node| node.get("op").and_then(Value::as_str) == Some("Placeholder"))
        && let Some(dimensions) = placeholder
            .pointer("/attr/shape/shape/dim")
            .or_else(|| placeholder.pointer("/attr/_output_shapes/list/shape/0/dim"))
            .and_then(Value::as_array)
    {
        let mut shape = Vec::with_capacity(dimensions.len());
        for dimension in dimensions {
            let raw = dimension.get("size")?;
            let size = raw.as_u64().or_else(|| {
                let value = raw.as_str()?;
                if value == "-1" {
                    Some(0)
                } else {
                    value.parse::<u64>().ok()
                }
            })?;
            shape.push(size);
        }
        return Some(shape);
    }
    fn nested_shape(value: &Value) -> Option<Vec<u64>> {
        match value {
            Value::Object(object) => {
                for key in ["batch_input_shape", "batchInputShape", "input_shape", "inputShape"] {
                    if let Some(shape) = object.get(key).and_then(Value::as_array) {
                        let values = shape
                            .iter()
                            .map(|value| value.as_u64().unwrap_or(0))
                            .collect::<Vec<_>>();
                        if !values.is_empty() {
                            return Some(values);
                        }
                    }
                }
                object.values().find_map(nested_shape)
            }
            Value::Array(values) => values.iter().find_map(nested_shape),
            _ => None,
        }
    }
    nested_shape(value)
}

fn expected_weights_bytes(model_json: &[u8]) -> Result<Option<u64>, String> {
    let value: Value = serde_json::from_slice(model_json)
        .map_err(|error| format!("model.json 格式无效：{error}"))?;
    let mut total = 0u64;
    let mut found = false;
    for weight in value
        .get("weightsManifest")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|manifest| manifest.get("weights").and_then(Value::as_array))
        .flatten()
    {
        found = true;
        let element_bytes = match weight.get("dtype").and_then(Value::as_str) {
            Some("float32" | "int32") => 4u64,
            Some("bool" | "uint8" | "int8") => 1u64,
            Some(dtype) => return Err(format!("模型包含不支持的权重类型：{dtype}")),
            None => return Err("模型权重缺少 dtype".to_string()),
        };
        let shape = weight
            .get("shape")
            .and_then(Value::as_array)
            .ok_or_else(|| "模型权重缺少 shape".to_string())?;
        let elements = shape.iter().try_fold(1u64, |product, dimension| {
            let dimension = dimension
                .as_u64()
                .ok_or_else(|| "模型权重 shape 无效".to_string())?;
            product
                .checked_mul(dimension)
                .ok_or_else(|| "模型权重大小溢出".to_string())
        })?;
        total = total
            .checked_add(
                elements
                    .checked_mul(element_bytes)
                    .ok_or_else(|| "模型权重大小溢出".to_string())?,
            )
            .ok_or_else(|| "模型权重总大小溢出".to_string())?;
    }
    Ok(found.then_some(total))
}

fn validate_model_pair(
    model_json: &[u8],
    weights: &[u8],
    identity: KnownModelIdentity,
    archive: bool,
) -> Result<(), String> {
    validate_model_json(model_json, identity, archive)?;
    if weights.is_empty() {
        return Err("模型权重为空".to_string());
    }
    if let Some(expected) = expected_weights_bytes(model_json)?
        && weights.len() as u64 != expected
    {
        return Err(format!(
            "模型权重大小不匹配：应为 {expected} 字节，实际为 {} 字节",
            weights.len()
        ));
    }
    Ok(())
}

fn validate_musicnn_topology(value: &Value) -> Result<(), String> {
    let nodes = value
        .pointer("/modelTopology/node")
        .and_then(Value::as_array)
        .ok_or_else(|| "MusiCNN model.json 缺少 graph 节点".to_string())?;
    let names: HashSet<&str> = nodes
        .iter()
        .filter_map(|node| node.get("name").and_then(Value::as_str))
        .collect();
    for required in [
        "model/Placeholder",
        "model/dense/Relu",
        "model/Sigmoid",
    ] {
        if !names.contains(required) {
            return Err(format!("MusiCNN model.json 缺少已知节点：{required}"));
        }
    }
    Ok(())
}

fn model_topology_contains_node(value: &Value, expected_name: &str) -> bool {
    value
        .pointer("/modelTopology/node")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|node| node.get("name").and_then(Value::as_str) == Some(expected_name))
}

fn model_output_node_matches(value: &Value, identity: &str, expected_name: &str) -> bool {
    if model_topology_contains_node(value, expected_name) {
        return true;
    }
    // The official Essentia TFJS directories for emoMusic and MuSe expose
    // the same regression tensor as `dense_out`, while their PB/ONNX
    // metadata uses `model/Identity`.
    matches!(
        (identity, expected_name),
        ("emomusic" | "muse", "model/Identity")
            if model_topology_contains_node(value, "dense_out")
    ) || matches!(
        (identity, expected_name),
        ("emomusic" | "muse", "dense_out")
            if model_topology_contains_node(value, "model/Identity")
    )
}

fn model_output_units(value: &Value, preferred_name: Option<&str>) -> Option<u64> {
    if let Some(name) = preferred_name
        && let Some(node) = value
            .pointer("/modelTopology/node")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|node| node.get("name").and_then(Value::as_str) == Some(name))
        && let Some(units) = model_shape_last_dimension(node.pointer("/attr/_output_shapes"))
    {
        return Some(units);
    }
    let weights = value
        .get("weightsManifest")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|manifest| manifest.get("weights").and_then(Value::as_array))
        .flatten()
        .collect::<Vec<_>>();
    for marker in ["dense_out/", "dense_2/", "dense_1/"] {
        if let Some(units) = weights.iter().rev().find_map(|weight| {
            let name = weight.get("name")?.as_str()?;
            if !name.contains(marker) {
                return None;
            }
            weight.get("shape")?.as_array()?.last()?.as_u64()
        }) {
            return Some(units);
        }
    }
    match value {
        Value::Object(object) => {
            if let Some(units) = object.get("units").and_then(Value::as_u64) {
                return Some(units);
            }
            object
                .values()
                .rev()
                .find_map(|value| model_output_units(value, None))
        }
        Value::Array(values) => values
            .iter()
            .rev()
            .find_map(|value| model_output_units(value, None)),
        _ => None,
    }
}

fn model_shape_last_dimension(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    match value {
        Value::Object(object) => {
            if let Some(dimensions) = object.get("dim").and_then(Value::as_array) {
                return dimensions.last().and_then(|dimension| {
                    let size = dimension.get("size")?;
                    size.as_u64()
                        .or_else(|| size.as_str()?.parse::<u64>().ok())
                });
            }
            object.values().rev().find_map(|value| model_shape_last_dimension(Some(value)))
        }
        Value::Array(values) => values
            .iter()
            .rev()
            .find_map(|value| model_shape_last_dimension(Some(value))),
        _ => None,
    }
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.contains('\0') {
        return Err("权重路径为空或包含非法字符".to_string());
    }
    let normalized = path.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
        })
    {
        return Err("权重路径必须位于模型文件目录内".to_string());
    }
    Ok(())
}

fn install_artifacts(models_path: &Path, artifacts: ModelArtifacts) -> Result<(), String> {
    let identity = KNOWN_MODELS
        .iter()
        .find(|identity| identity.id == artifacts.id)
        .copied()
        .ok_or_else(|| "未知的 Essentia 模型身份".to_string())?;
    validate_model_pair(&artifacts.model_json, &artifacts.weights, identity, false)?;
    install_pair_with_rollback(models_path, artifacts.id, &artifacts.model_json, &artifacts.weights)
}

pub fn install_bundled_model_set(
    bundled_path: &Path,
    models_path: &Path,
    overwrite: bool,
) -> Result<Vec<String>, String> {
    let mut artifacts = Vec::with_capacity(KNOWN_MODELS.len());
    let mut total_bytes = 0u64;
    for identity in KNOWN_MODELS.iter().copied() {
        let json_name = format!("{}.json", identity.id);
        let weights_name = format!("{}.bin", identity.id);
        let json_path = bundled_path.join(&json_name);
        let weights_path = bundled_path.join(&weights_name);
        if is_optional_bundled_model(identity.id)
            && (!json_path.is_file() || !weights_path.is_file())
        {
            // New emotion heads are optional until their official, strictly
            // validated offline resources are available. Missing optional
            // pairs must not prevent ordinary conversion or legacy analysis.
            continue;
        }
        let model_json = read_bundled_model_file(&json_path, &json_name)?;
        let weights = read_bundled_model_file(&weights_path, &weights_name)?;
        total_bytes = total_bytes
            .checked_add((model_json.len() + weights.len()) as u64)
            .ok_or_else(|| "内置模型总大小超出限制".to_string())?;
        if total_bytes > MAX_MODEL_BATCH_BYTES {
            return Err(format!("内置模型总大小超过 {MAX_MODEL_BATCH_BYTES} 字节"));
        }
        validate_model_pair(&model_json, &weights, identity, false)
            .map_err(|error| format!("内置模型 {json_name} 无效：{error}"))?;
        artifacts.push(ModelArtifacts {
            id: identity.id,
            model_json,
            weights,
        });
    }

    fs::create_dir_all(models_path).map_err(|error| format!("创建 Essentia 模型目录失败：{error}"))?;
    let mut installed_ids = Vec::new();
    for artifacts in artifacts {
        let identity = KNOWN_MODELS
            .iter()
            .find(|identity| identity.id == artifacts.id)
            .copied()
            .expect("bundled model identity is defined");
        if !overwrite && installed_model_pair_matches_identity(models_path, identity) {
            continue;
        }
        let id = artifacts.id;
        install_artifacts(models_path, artifacts)?;
        installed_ids.push(id.to_string());
    }
    Ok(installed_ids)
}

fn is_optional_bundled_model(id: &str) -> bool {
    matches!(
        id,
        "emomusic"
            | "muse"
            | "mirex"
            | "discogs_effnet"
            | "discogs_effnet_embedding"
            | "genre_discogs400"
            | "discogs_mood_theme"
            | "discogs_approachability"
            | "discogs_instrumentation"
            | "discogs_timbre"
            | "discogs_danceability"
    )
}

fn read_bundled_model_file(path: &Path, file_name: &str) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("读取内置模型 {file_name} 失败：{error}"))?;
    if !metadata.is_file() {
        return Err(format!("内置模型 {file_name} 不是普通文件"));
    }
    if metadata.len() > MAX_MODEL_FILE_BYTES {
        return Err(format!("内置模型 {file_name} 超过 {MAX_MODEL_FILE_BYTES} 字节"));
    }
    fs::read(path).map_err(|error| format!("读取内置模型 {file_name} 失败：{error}"))
}

fn installed_model_pair_matches_identity(models_path: &Path, identity: KnownModelIdentity) -> bool {
    let json_path = models_path.join(format!("{}.json", identity.id));
    let weights_path = models_path.join(format!("{}.bin", identity.id));
    let Ok(model_json) = fs::read(json_path) else {
        return false;
    };
    let Ok(weights) = fs::read(weights_path) else {
        return false;
    };
    validate_model_pair(&model_json, &weights, identity, false).is_ok()
}

pub fn installed_model_pair_is_valid(models_path: &Path, id: &str) -> bool {
    KNOWN_MODELS
        .iter()
        .find(|identity| identity.id == id)
        .copied()
        .is_some_and(|identity| installed_model_pair_matches_identity(models_path, identity))
}

pub fn install_pair_with_rollback(
    models_path: &Path,
    id: &str,
    model_json: &[u8],
    weights: &[u8],
) -> Result<(), String> {
    if model_json.is_empty() || weights.is_empty() {
        return Err("模型结构或权重为空".to_string());
    }
    fs::create_dir_all(models_path).map_err(|error| format!("创建模型目录失败：{error}"))?;
    let token = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("获取模型安装时间失败：{error}"))?
            .as_nanos()
    );
    let staged_json = models_path.join(format!(".w4dj-import-{token}-{id}.json"));
    let staged_bin = models_path.join(format!(".w4dj-import-{token}-{id}.bin"));
    let backup_json = models_path.join(format!(".w4dj-backup-{token}-{id}.json"));
    let backup_bin = models_path.join(format!(".w4dj-backup-{token}-{id}.bin"));
    fs::write(&staged_json, model_json).map_err(|error| format!("暂存模型结构失败：{error}"))?;
    if let Err(error) = fs::write(&staged_bin, weights) {
        let _ = fs::remove_file(&staged_json);
        return Err(format!("暂存模型权重失败：{error}"));
    }
    let staged_validation = (|| {
        let staged_model_json =
            fs::read(&staged_json).map_err(|error| format!("重新读取暂存模型结构失败：{error}"))?;
        let staged_weights =
            fs::read(&staged_bin).map_err(|error| format!("重新读取暂存模型权重失败：{error}"))?;
        if staged_model_json != model_json || staged_weights != weights {
            return Err("暂存模型重新读取后内容不一致".to_string());
        }
        let identity = KNOWN_MODELS
            .iter()
            .find(|identity| identity.id == id)
            .copied()
            .ok_or_else(|| "未知的 Essentia 模型身份".to_string())?;
        validate_model_pair(&staged_model_json, &staged_weights, identity, false)?;
        Ok::<(), String>(())
    })();
    if let Err(error) = staged_validation {
        cleanup_import_files(&[&staged_json, &staged_bin]);
        return Err(error);
    }

    let target_json = models_path.join(format!("{id}.json"));
    let target_bin = models_path.join(format!("{id}.bin"));
    let had_json = target_json.is_file();
    let had_bin = target_bin.is_file();
    if had_json
        && let Err(error) = fs::rename(&target_json, &backup_json)
    {
        cleanup_import_files(&[&staged_json, &staged_bin]);
        return Err(format!("备份模型结构失败：{error}"));
    }
    if had_bin
        && let Err(error) = fs::rename(&target_bin, &backup_bin)
    {
        restore_import_backup(&target_json, &backup_json, had_json);
        cleanup_import_files(&[&staged_json, &staged_bin]);
        return Err(format!("备份模型权重失败：{error}"));
    }

    let install_result = (|| {
        fs::rename(&staged_json, &target_json).map_err(|error| format!("安装模型结构失败：{error}"))?;
        fs::rename(&staged_bin, &target_bin).map_err(|error| format!("安装模型权重失败：{error}"))?;
        Ok::<(), String>(())
    })();
    if let Err(error) = install_result {
        cleanup_import_files(&[&target_json, &target_bin, &staged_json, &staged_bin]);
        restore_import_backup(&target_json, &backup_json, had_json);
        restore_import_backup(&target_bin, &backup_bin, had_bin);
        return Err(error);
    }
    cleanup_import_files(&[&backup_json, &backup_bin]);
    Ok(())
}

fn restore_import_backup(target: &Path, backup: &Path, existed: bool) {
    if existed && backup.is_file() {
        let _ = fs::rename(backup, target);
    }
}

fn cleanup_import_files(paths: &[&Path]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

pub fn extract_official_musicnn_pair(archive: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let entries = extract_model_archive(archive)?;
    let model_entries: Vec<&ArchiveEntry> = entries
        .iter()
        .filter(|entry| {
            Path::new(&entry.name)
                .file_name()
                .and_then(|name| name.to_str())
                == Some("model.json")
        })
        .collect();
    if model_entries.len() != 1 {
        return Err(if model_entries.is_empty() {
            "Essentia MusiCNN 模型包缺少 model.json".to_string()
        } else {
            "Essentia MusiCNN 模型包包含多个冲突的 model.json".to_string()
        });
    }
    let model_entry = model_entries[0];
    let model_parent = Path::new(&model_entry.name)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let manifest_paths = validate_model_json(
        &model_entry.data,
        *KNOWN_MODELS
            .iter()
            .find(|identity| identity.id == "musicnn_embedding")
            .expect("embedding identity is defined"),
        true,
    )?;
    let mut weights = Vec::new();
    for path in manifest_paths {
        let expected = model_parent.join(&path);
        let entry = entries
            .iter()
            .find(|entry| Path::new(&entry.name) == expected);
        let Some(entry) = entry else {
            return Err(format!("Essentia MusiCNN 模型包缺少权重：{path}"));
        };
        weights.extend_from_slice(&entry.data);
    }
    if weights.is_empty() {
        return Err("Essentia MusiCNN 模型包权重为空".to_string());
    }
    Ok((model_entry.data.clone(), weights))
}

fn extract_model_archive(archive: &[u8]) -> Result<Vec<ArchiveEntry>, String> {
    let mut entries = Vec::new();
    let mut seen_names = HashSet::new();
    let mut cursor = 0usize;
    let mut expanded_total = 0usize;
    while let Some(relative) = archive[cursor..]
        .windows(4)
        .position(|window| window == b"PK\x03\x04")
    {
        let offset = cursor + relative;
        if offset + 30 > archive.len() {
            return Err("Essentia 模型包的条目头损坏".to_string());
        }
        let flags = read_u16(archive, offset + 6)?;
        if flags & 0x0001 != 0 || flags & 0x0008 != 0 {
            return Err("Essentia 模型包使用了不支持的加密或数据描述符".to_string());
        }
        let compression = read_u16(archive, offset + 8)?;
        let compressed_size = read_u32(archive, offset + 18)? as usize;
        let uncompressed_size = read_u32(archive, offset + 22)? as usize;
        let name_length = read_u16(archive, offset + 26)? as usize;
        let extra_length = read_u16(archive, offset + 28)? as usize;
        let name_start = offset + 30;
        let name_end = name_start
            .checked_add(name_length)
            .ok_or_else(|| "Essentia 模型包的文件名长度无效".to_string())?;
        let data_floor = name_end
            .checked_add(extra_length)
            .ok_or_else(|| "Essentia 模型包的扩展字段长度无效".to_string())?;
        if data_floor > archive.len() {
            return Err("Essentia 模型包的条目头损坏".to_string());
        }
        let name = std::str::from_utf8(&archive[name_start..name_end])
            .map_err(|_| "Essentia 模型包包含无效文件名".to_string())?
            .replace('\\', "/");
        validate_archive_name(&name)?;
        if !seen_names.insert(name.clone()) {
            return Err("Essentia 模型包包含重复文件".to_string());
        }
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            return Err("Essentia 模型包条目数量超出限制".to_string());
        }
        if compressed_size == 0 || uncompressed_size == 0 {
            return Err("Essentia 模型包包含空文件".to_string());
        }
        expanded_total = expanded_total
            .checked_add(uncompressed_size)
            .ok_or_else(|| "Essentia 模型包展开大小超出限制".to_string())?;
        if expanded_total > MAX_ARCHIVE_OUTPUT_BYTES {
            return Err("Essentia 模型包展开大小超出限制".to_string());
        }

        let (data_start, data) = decode_archive_entry(
            archive,
            data_floor,
            compressed_size,
            uncompressed_size,
            compression,
        )?;
        entries.push(ArchiveEntry { name, data });
        cursor = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| "Essentia 模型包长度溢出".to_string())?;
        if cursor > archive.len() {
            return Err("Essentia 模型包的数据长度无效".to_string());
        }
    }
    if entries.is_empty() {
        return Err("Essentia MusiCNN 模型包缺少可读取条目".to_string());
    }
    Ok(entries)
}

fn decode_archive_entry(
    archive: &[u8],
    data_floor: usize,
    compressed_size: usize,
    uncompressed_size: usize,
    compression: u16,
) -> Result<(usize, Vec<u8>), String> {
    let max_start = data_floor
        .saturating_add(MAX_MALFORMED_DATA_OFFSET)
        .min(archive.len().saturating_sub(compressed_size));
    for data_start in data_floor..=max_start {
        let data_end = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| "Essentia 模型包数据长度溢出".to_string())?;
        if data_end > archive.len() {
            break;
        }
        let mut output = Vec::with_capacity(uncompressed_size.min(MAX_ARCHIVE_OUTPUT_BYTES));
        let decoded = match compression {
            0 => {
                output.extend_from_slice(&archive[data_start..data_end]);
                true
            }
            8 => DeflateDecoder::new(&archive[data_start..data_end])
                .take(uncompressed_size.saturating_add(1) as u64)
                .read_to_end(&mut output)
                .is_ok(),
            _ => return Err("Essentia 模型包使用了不支持的压缩方式".to_string()),
        };
        if decoded && output.len() == uncompressed_size {
            return Ok((data_start, output));
        }
    }
    Err("Essentia 模型包的压缩数据无法读取".to_string())
}

fn validate_archive_name(name: &str) -> Result<(), String> {
    let path = Path::new(name);
    if name.is_empty()
        || path.components().any(|component| {
            matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
        })
        || name.to_ascii_lowercase().ends_with(".zip")
    {
        return Err("Essentia 模型包包含不安全文件路径".to_string());
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "Essentia 模型包条目头损坏".to_string())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "Essentia 模型包条目头损坏".to_string())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn extension_is(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn display_file_name(path: &Path) -> String {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "模型文件".to_string())
}

fn display_relative_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "权重文件".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestFixture {
        root: PathBuf,
        input_dir: PathBuf,
        models_dir: PathBuf,
        inputs: Vec<PathBuf>,
    }

    impl TestFixture {
        fn new(folder: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "w4dj-essentia-import-{}-{}",
                std::process::id(),
                FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let input_dir = root.join(folder);
            let models_dir = root.join("models");
            fs::create_dir_all(&input_dir).unwrap();
            fs::create_dir_all(&models_dir).unwrap();
            Self {
                root,
                input_dir,
                models_dir,
                inputs: Vec::new(),
            }
        }

        fn with_installed_pair(id: &str, json: &[u8], bin: &[u8]) -> Self {
            let fixture = Self::new("existing");
            fs::write(fixture.models_dir.join(format!("{id}.json")), json).unwrap();
            fs::write(fixture.models_dir.join(format!("{id}.bin")), bin).unwrap();
            fixture
        }

        fn write_model_json(&mut self, paths: &[&str]) {
            self.write_named_json("model.json", paths);
        }

        fn write_named_json(&mut self, name: &str, paths: &[&str]) {
            let path = self.input_dir.join(name);
            let units = if self.input_dir.to_string_lossy().contains("genre") {
                8
            } else {
                2
            };
            let manifest = serde_json::json!({
                "modelTopology": { "model_config": { "config": { "layers": [
                    { "config": { "batch_input_shape": [null, 200], "units": units } }
                ] } } },
                "weightsManifest": [{ "paths": paths, "weights": [] }]
            });
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            self.inputs.push(path);
        }

        fn write_weight(&mut self, name: &str, data: &[u8]) {
            let path = self.input_dir.join(name);
            fs::write(&path, data).unwrap();
            self.inputs.push(path);
        }

        fn input_paths(&self) -> Vec<PathBuf> {
            self.inputs.clone()
        }

        fn write_zip(&mut self, name: &str, entries: &[(&str, &[u8])]) {
            let path = self.input_dir.join(name);
            fs::write(&path, stored_zip(entries)).unwrap();
            self.inputs.push(path);
        }

        fn models_path(&self) -> &Path {
            &self.models_dir
        }
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn imports_loose_known_tensorflow_model_and_weight() {
        let mut fixture = TestFixture::new("mood_happy-msd-musicnn-1-tfjs");
        fixture.write_model_json(&["group1-shard1of1.bin"]);
        fixture.write_weight("group1-shard1of1.bin", b"weights");
        let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
        assert_eq!(report.installed_ids, ["mood_happy"], "issues: {:?}", report.issues);
        assert!(report.issues.is_empty());
        assert_eq!(
            fs::read(fixture.models_path().join("mood_happy.bin")).unwrap(),
            b"weights"
        );
    }

    #[test]
    fn imports_emomusic_muse_and_mirex_with_distinct_output_nodes() {
        let cases = [
            ("emomusic-msd-musicnn-2-tfjs", "emomusic", 2, "model/Identity"),
            ("muse-msd-musicnn-2-tfjs", "muse", 2, "model/Identity"),
            ("moods_mirex-msd-musicnn-1-tfjs", "mirex", 5, "PartitionedCall"),
        ];
        for (folder, id, units, output_name) in cases {
            let mut fixture = TestFixture::new(folder);
            let model_path = fixture.input_dir.join("model.json");
            let model_json = serde_json::json!({
                "modelTopology": {
                    "node": [{"name": output_name}],
                    "model_config": {"config": {"layers": [{"config": {
                        "batch_input_shape": [null, 200], "units": units
                    }}]}}
                },
                "weightsManifest": [{"paths": ["group1-shard1of1.bin"], "weights": []}]
            });
            fs::write(&model_path, serde_json::to_vec(&model_json).unwrap()).unwrap();
            let weights_path = fixture.input_dir.join("group1-shard1of1.bin");
            fs::write(&weights_path, b"emotion-weights").unwrap();
            fixture.inputs.extend([model_path, weights_path]);
            let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
            assert_eq!(report.installed_ids, [id], "issues: {:?}", report.issues);
            assert!(report.issues.is_empty());
        }
    }

    #[test]
    fn rejects_emotion_head_with_wrong_output_node_or_width() {
        for (folder, output_name, units) in [
            ("emomusic-msd-musicnn-2-tfjs", "PartitionedCall", 2),
            ("moods_mirex-msd-musicnn-1-tfjs", "PartitionedCall", 2),
        ] {
            let mut fixture = TestFixture::new(folder);
            let model_path = fixture.input_dir.join("model.json");
            let model_json = serde_json::json!({
                "modelTopology": {
                    "node": [{"name": output_name}],
                    "model_config": {"config": {"layers": [{"config": {
                        "batch_input_shape": [null, 128], "units": units
                    }}]}}
                },
                "weightsManifest": [{"paths": ["weights.bin"], "weights": []}]
            });
            fs::write(&model_path, serde_json::to_vec(&model_json).unwrap()).unwrap();
            let weights_path = fixture.input_dir.join("weights.bin");
            fs::write(&weights_path, b"bad").unwrap();
            fixture.inputs.extend([model_path, weights_path]);
            let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
            assert!(report.installed_ids.is_empty());
            assert!(report.issues.iter().any(|issue| issue.reason.contains("输出") || issue.reason.contains("输入")));
        }
    }

    #[test]
    fn accepts_official_graph_model_output_shape() {
        let identity = KNOWN_MODELS
            .iter()
            .find(|identity| identity.id == "mood_happy")
            .copied()
            .unwrap();
        let model_json = serde_json::to_vec(&serde_json::json!({
            "modelTopology": { "node": [] },
            "weightsManifest": [{
                "paths": ["group1-shard1of1.bin"],
                "weights": [
                    { "name": "dense/kernel", "shape": [200, 100], "dtype": "float32" },
                    { "name": "dense_1/kernel", "shape": [100, 2], "dtype": "float32" }
                ]
            }]
        }))
        .unwrap();
        assert!(validate_model_json(&model_json, identity, false).is_ok());
    }

    #[test]
    fn rejects_classifier_with_wrong_embedding_width() {
        let identity = KNOWN_MODELS
            .iter()
            .find(|identity| identity.id == "mood_happy")
            .copied()
            .unwrap();
        let model_json = serde_json::to_vec(&serde_json::json!({
            "modelTopology": { "node": [] },
            "weightsManifest": [{
                "paths": ["group1-shard1of1.bin"],
                "weights": [
                    { "name": "dense/kernel", "shape": [128, 100], "dtype": "float32" },
                    { "name": "dense_1/kernel", "shape": [100, 2], "dtype": "float32" }
                ]
            }]
        }))
        .unwrap();
        assert!(validate_model_json(&model_json, identity, false).is_err());
    }

    #[test]
    fn missing_weight_does_not_replace_an_installed_model() {
        let mut fixture = TestFixture::with_installed_pair("mood_happy", b"old-json", b"old-bin");
        fixture.write_named_json("mood_happy-msd-musicnn-1-tfjs/model.json", &["missing.bin"]);
        let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
        assert!(report.installed_ids.is_empty());
        assert!(report.issues.iter().any(|issue| issue.reason.contains("缺少权重")));
        assert_eq!(
            fs::read(fixture.models_path().join("mood_happy.bin")).unwrap(),
            b"old-bin"
        );
    }

    #[test]
    fn ambiguous_renamed_head_is_not_guessed() {
        let mut fixture = TestFixture::new("renamed-model");
        fixture.write_model_json(&["group1-shard1of1.bin"]);
        fixture.write_weight("group1-shard1of1.bin", b"weights");
        let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
        assert!(report.installed_ids.is_empty());
        assert!(report.issues.iter().any(|issue| issue.reason.contains("无法识别")));
    }

    #[test]
    fn rejects_invalid_relative_weight_path() {
        assert!(validate_relative_path("../model.bin").is_err());
        assert!(validate_relative_path("/tmp/model.bin").is_err());
        assert!(validate_relative_path("folder/model.bin").is_ok());
    }

    #[test]
    fn imports_official_musicnn_zip_layout() {
        let mut fixture = TestFixture::new("downloads");
        let model_json = musicnn_test_json("group1-shard1of1.bin");
        fixture.write_zip(
            "msd-musicnn-1-tfjs.zip",
            &[("model.json", &model_json), ("group1-shard1of1.bin", b"embedding")],
        );
        let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
        assert_eq!(report.installed_ids, ["musicnn_embedding"]);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn rejects_archive_path_traversal_duplicate_and_nested_entries() {
        for entries in [
            vec![("../model.json", b"bad".as_slice())],
            vec![("/model.json", b"bad".as_slice())],
            vec![("model.json", b"a".as_slice()), ("model.json", b"b".as_slice())],
            vec![("nested.zip", b"bad".as_slice())],
        ] {
            assert!(extract_model_archive(&stored_zip(&entries)).is_err());
        }
    }

    #[test]
    fn rejects_archive_expansion_over_limit_before_decoding() {
        let mut archive = Vec::new();
        append_stored_header(&mut archive, "large.bin", 1, (MAX_ARCHIVE_OUTPUT_BYTES + 1) as u32);
        archive.push(0);
        assert!(extract_model_archive(&archive).is_err());
    }

    #[test]
    fn rejects_musicnn_archive_without_known_graph_nodes() {
        let model_json = serde_json::to_vec(&serde_json::json!({
            "modelTopology": { "node": [] },
            "weightsManifest": [{ "paths": ["group1-shard1of1.bin"], "weights": [] }]
        }))
        .unwrap();
        let archive = stored_zip(&[
            ("model.json", &model_json),
            ("group1-shard1of1.bin", b"embedding"),
        ]);
        assert!(extract_official_musicnn_pair(&archive).is_err());
    }

    #[test]
    fn musicnn_archive_pairs_manifest_shards_by_exact_relative_path() {
        let model_json = musicnn_test_json("weights/group1-shard1of1.bin");
        let archive = stored_zip(&[
            ("bundle/model.json", &model_json),
            ("other/group1-shard1of1.bin", b"wrong"),
            ("bundle/weights/group1-shard1of1.bin", b"correct"),
        ]);
        let (_, weights) = extract_official_musicnn_pair(&archive).unwrap();
        assert_eq!(weights, b"correct");
    }

    #[test]
    fn rejects_musicnn_archive_with_multiple_model_json_files() {
        let model_json = musicnn_test_json("group1-shard1of1.bin");
        let archive = stored_zip(&[
            ("a/model.json", &model_json),
            ("a/group1-shard1of1.bin", b"a"),
            ("b/model.json", &model_json),
            ("b/group1-shard1of1.bin", b"b"),
        ]);
        let error = extract_official_musicnn_pair(&archive).unwrap_err();
        assert!(error.contains("多个冲突"));
    }

    #[test]
    fn conflicting_models_are_rejected_without_replacing_the_installed_pair() {
        let fixture = TestFixture::new("conflict");
        write_model_pair(&fixture.models_dir, "mood_happy", Some(2), "old");
        let mut inputs = Vec::new();
        for folder in ["a", "b"] {
            let dir = fixture
                .input_dir
                .join(folder)
                .join("mood_happy-msd-musicnn-1-tfjs");
            fs::create_dir_all(&dir).unwrap();
            let json = dir.join("model.json");
            let bin = dir.join("group1-shard1of1.bin");
            fs::write(
                &json,
                serde_json::to_vec(&serde_json::json!({
                    "modelTopology": { "model_config": { "config": { "layers": [
                        { "config": { "batch_input_shape": [null, 200], "units": 2 } }
                    ] } } },
                    "weightsManifest": [{ "paths": ["group1-shard1of1.bin"], "weights": [] }]
                }))
                .unwrap(),
            )
            .unwrap();
            fs::write(&bin, folder.as_bytes()).unwrap();
            inputs.extend([json, bin]);
        }

        let report = import_model_paths(&inputs, fixture.models_path()).unwrap();

        assert!(report.installed_ids.is_empty());
        assert!(report.issues.iter().any(|issue| issue.reason.contains("冲突版本")));
        assert_eq!(
            fs::read(fixture.models_dir.join("mood_happy.bin")).unwrap(),
            b"old-mood_happy"
        );
    }

    #[test]
    fn staged_pair_is_validated_before_replacing_the_installed_pair() {
        let fixture = TestFixture::new("staged-validation");
        write_model_pair(&fixture.models_dir, "mood_happy", Some(2), "old");

        let error = install_pair_with_rollback(
            fixture.models_path(),
            "mood_happy",
            br#"{"modelTopology":{},"weightsManifest":[]}"#,
            b"new",
        )
        .unwrap_err();

        assert!(error.contains("weightsManifest"));
        assert_eq!(
            fs::read(fixture.models_dir.join("mood_happy.bin")).unwrap(),
            b"old-mood_happy"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_model_input() {
        use std::os::unix::fs::symlink;
        let mut fixture = TestFixture::new("symlink");
        let real = fixture.input_dir.join("real.json");
        fs::write(&real, b"{}").unwrap();
        let link = fixture.input_dir.join("model.json");
        symlink(&real, &link).unwrap();
        fixture.inputs.push(link);
        let report = import_model_paths(&fixture.input_paths(), fixture.models_path()).unwrap();
        assert!(report.installed_ids.is_empty());
        assert!(report.issues.iter().any(|issue| issue.reason.contains("普通文件")));
    }

    #[test]
    fn installs_missing_bundled_models_without_overwriting_a_valid_local_pair() {
        let fixture = TestFixture::new("bundled-install");
        let bundled = fixture.root.join("bundled");
        write_bundled_model_set(&bundled, "bundled");
        write_model_pair(&fixture.models_dir, "mood_happy", Some(2), "local");

        let installed = install_bundled_model_set(&bundled, &fixture.models_dir, false).unwrap();

        assert_eq!(installed.len(), KNOWN_MODELS.len() - 1);
        assert_eq!(
            fs::read(fixture.models_dir.join("mood_happy.bin")).unwrap(),
            b"local-mood_happy"
        );
        assert!(fixture.models_dir.join("musicnn_embedding.json").is_file());
    }

    #[test]
    fn restoring_bundled_models_replaces_existing_pairs() {
        let fixture = TestFixture::new("bundled-restore");
        let bundled = fixture.root.join("bundled");
        write_bundled_model_set(&bundled, "bundled");
        write_model_pair(&fixture.models_dir, "mood_happy", Some(2), "local");

        let installed = install_bundled_model_set(&bundled, &fixture.models_dir, true).unwrap();

        assert_eq!(installed.len(), KNOWN_MODELS.len());
        assert_eq!(
            fs::read(fixture.models_dir.join("mood_happy.bin")).unwrap(),
            b"bundled-mood_happy"
        );
    }

    #[test]
    fn incomplete_bundled_set_is_rejected_before_installing_any_pair() {
        let fixture = TestFixture::new("bundled-incomplete");
        let bundled = fixture.root.join("bundled");
        write_bundled_model_set(&bundled, "bundled");
        fs::remove_file(bundled.join("mood_happy.bin")).unwrap();

        let error = install_bundled_model_set(&bundled, &fixture.models_dir, false).unwrap_err();

        assert!(error.contains("mood_happy.bin"));
        assert!(!fixture.models_dir.join("musicnn_embedding.json").exists());
    }

    #[test]
    fn checked_in_bundled_models_are_complete_and_valid() {
        let fixture = TestFixture::new("bundled-resource-validation");
        let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("essentia-models");
        let installed = install_bundled_model_set(&resources, fixture.models_path(), true)
            .expect("checked-in model resources must be installable");
        let available = KNOWN_MODELS
            .iter()
            .filter(|identity| {
                resources.join(format!("{}.json", identity.id)).is_file()
                    && resources.join(format!("{}.bin", identity.id)).is_file()
            })
            .count();
        assert_eq!(installed.len(), available);
        assert!(installed.contains(&"musicnn_embedding".to_string()));
    }

    #[test]
    fn checked_in_discogs_models_have_explicit_shapes_and_outputs() {
        let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("essentia-models");
        for id in [
            "discogs_effnet_embedding",
            "genre_discogs400",
            "discogs_mood_theme",
            "discogs_approachability",
            "discogs_instrumentation",
            "discogs_timbre",
            "discogs_danceability",
        ] {
            let identity = KNOWN_MODELS
                .iter()
                .find(|identity| identity.id == id)
                .copied()
                .expect("Discogs identity is registered");
            let model_json = fs::read(resources.join(format!("{id}.json"))).unwrap();
            let weights = fs::read(resources.join(format!("{id}.bin"))).unwrap();
            validate_model_pair(&model_json, &weights, identity, false).unwrap();
            let value: Value = serde_json::from_slice(&model_json).unwrap();
            assert!(model_output_node_matches(
                &value,
                id,
                identity.expected_output_name.unwrap(),
            ));
            if let Some(shape) = identity.expected_input_shape {
                assert_eq!(model_input_shape(&value).as_deref(), Some(shape));
            } else {
                assert_eq!(model_input_units(&value), identity.expected_input_width);
            }
        }
    }

    #[test]
    fn bundled_discogs_resources_use_single_canonical_embedding_pair() {
        let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("essentia-models");
        assert!(resources.join("discogs_effnet_embedding.json").is_file());
        assert!(resources.join("discogs_effnet_embedding.bin").is_file());
        assert!(!resources.join("discogs_effnet.json").exists());
        assert!(!resources.join("discogs_effnet.bin").exists());
        let embedding_json: Value = serde_json::from_slice(
            &fs::read(resources.join("discogs_effnet_embedding.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            embedding_json["weightsManifest"][0]["paths"][0].as_str(),
            Some("discogs_effnet_embedding.bin")
        );
    }

    #[test]
    fn legacy_discogs_embedding_identifier_remains_importable() {
        let fixture = TestFixture::new("discogs_effnet");
        write_model_pair(&fixture.input_dir, "discogs_effnet", Some(1280), "legacy");
        let inputs = vec![
            fixture.input_dir.join("discogs_effnet.json"),
            fixture.input_dir.join("discogs_effnet.bin"),
        ];
        let report = import_model_paths(&inputs, fixture.models_path()).unwrap();
        assert_eq!(report.installed_ids, ["discogs_effnet"]);
        assert!(report.issues.is_empty());
        assert!(installed_model_pair_is_valid(
            fixture.models_path(),
            "discogs_effnet"
        ));
    }

    fn write_bundled_model_set(path: &Path, marker: &str) {
        fs::create_dir_all(path).unwrap();
        for identity in KNOWN_MODELS {
            write_model_pair(path, identity.id, identity.expected_output_units, marker);
        }
    }

        fn write_model_pair(path: &Path, id: &str, units: Option<u64>, marker: &str) {
            fs::create_dir_all(path).unwrap();
        let mut topology = if id == "musicnn_embedding" {
            serde_json::json!({ "node": [
                { "name": "model/Placeholder" },
                { "name": "model/dense/Relu" },
                { "name": "model/Sigmoid", "attr": {
                    "_output_shapes": { "list": { "shape": [{ "dim": [
                        { "size": "-1" }, { "size": units.unwrap_or_default().to_string() }
                    ] }] } }
                } }
            ] })
        } else if id == "discogs_effnet" || id == "discogs_effnet_embedding" {
            serde_json::json!({ "node": [
                { "name": "discogs_input", "op": "Placeholder", "attr": {
                    "shape": { "shape": { "dim": [
                        { "size": "64" }, { "size": "128" }, { "size": "96" }
                    ] } }
                } },
                { "name": "discogs_embedding", "op": "Identity", "attr": {
                    "_output_shapes": { "list": { "shape": [{ "dim": [
                        { "size": "64" }, { "size": "1280" }
                    ] }] } }
                } }
            ] })
        } else if id == "genre_discogs400" {
            serde_json::json!({ "node": [
                { "name": "discogs_input", "op": "Placeholder", "attr": {
                    "shape": { "shape": { "dim": [
                        { "size": "-1" }, { "size": "1280" }
                    ] } }
                } },
                { "name": "discogs_genre", "op": "Identity", "attr": {
                    "_output_shapes": { "list": { "shape": [{ "dim": [
                        { "size": "-1" }, { "size": "400" }
                    ] }] } }
                } }
            ] })
        } else {
            serde_json::json!({ "model_config": { "config": { "layers": [] } } })
        };
        if let Some(units) = units && !matches!(
            id,
            "discogs_effnet" | "discogs_effnet_embedding" | "genre_discogs400"
        ) {
            topology["model_config"]["config"]["layers"] =
                serde_json::json!([{ "config": {
                    "batch_input_shape": [null, 200],
                    "units": units
                } }]);
        }
        if id == "emomusic" || id == "muse" {
            topology["node"] = serde_json::json!([{ "name": "model/Identity" }]);
        } else if id == "mirex" {
            topology["node"] = serde_json::json!([{ "name": "PartitionedCall" }]);
        } else if matches!(
            id,
            "discogs_mood_theme" | "discogs_instrumentation"
        ) {
            topology["node"] = serde_json::json!([
                { "name": "discogs_input", "op": "Placeholder", "attr": {
                    "shape": { "shape": { "dim": [{ "size": "-1" }, { "size": "1280" }] } }
                } },
                { "name": "model/Sigmoid" }
            ]);
        } else if matches!(
            id,
            "discogs_approachability" | "discogs_timbre" | "discogs_danceability"
        ) {
            topology["node"] = serde_json::json!([
                { "name": "discogs_input", "op": "Placeholder", "attr": {
                    "shape": { "shape": { "dim": [{ "size": "-1" }, { "size": "1280" }] } }
                } },
                { "name": "model/Softmax", "attr": {
                    "_output_shapes": { "list": { "shape": [{ "dim": [
                        { "size": "-1" }, { "size": units.unwrap_or_default().to_string() }
                    ] }] } }
                } }
            ]);
        }
        let manifest = serde_json::json!({
            "modelTopology": topology,
            "weightsManifest": [{ "paths": [format!("{id}.bin")], "weights": [] }]
        });
        fs::write(
            path.join(format!("{id}.json")),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(path.join(format!("{id}.bin")), format!("{marker}-{id}")).unwrap();
    }

    fn musicnn_test_json(weight_path: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "modelTopology": { "node": [
                { "name": "model/Placeholder" },
                { "name": "model/dense/Relu" },
                { "name": "model/Sigmoid" }
            ] },
            "weightsManifest": [{ "paths": [weight_path], "weights": [] }]
        }))
        .unwrap()
    }

    fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = Vec::new();
        for (name, data) in entries {
            append_stored_header(&mut archive, name, data.len() as u32, data.len() as u32);
            archive.extend_from_slice(data);
        }
        archive
    }

    fn append_stored_header(archive: &mut Vec<u8>, name: &str, compressed: u32, uncompressed: u32) {
        archive.extend_from_slice(b"PK\x03\x04");
        archive.extend_from_slice(&20u16.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&0u32.to_le_bytes());
        archive.extend_from_slice(&compressed.to_le_bytes());
        archive.extend_from_slice(&uncompressed.to_le_bytes());
        archive.extend_from_slice(&(name.len() as u16).to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(name.as_bytes());
    }
}
