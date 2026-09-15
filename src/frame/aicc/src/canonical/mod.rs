use buckyos_api::{CanonicalFieldRequirement, VoiceGender, VoiceSpec, VoiceStyle};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CanonicalFieldConverter {
    Passthrough,
    Prompt,
    OpenaiTtsVoiceV1,
    OpenaiTtsVoiceV2,
    GeminiTtsVoiceV1,
    GlmTtsVoiceV1,
    MinimaxTtsVoiceV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanonicalFieldMapping {
    pub converter: CanonicalFieldConverter,
    #[serde(default)]
    pub fallback: CanonicalFallback,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CanonicalFallback {
    #[default]
    Reject,
    Omit,
    Default {
        value: Value,
    },
}

impl CanonicalFieldMapping {
    #[cfg(test)]
    pub(crate) fn required(converter: CanonicalFieldConverter) -> Self {
        Self {
            converter,
            fallback: CanonicalFallback::Reject,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if let CanonicalFallback::Default { value } = &self.fallback {
            let requirement = CanonicalFieldRequirement::new(value.clone());
            let converted = convert_canonical_field(&self.converter, &requirement);
            if converted.quality == CanonicalMatchQuality::Unsupported
                || converted.resolution.is_none()
            {
                return Err("default value cannot be converted by the configured converter");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum CanonicalMatchQuality {
    Unsupported,
    Prompt,
    Default,
    Fuzzy,
    Exact,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CanonicalResolution {
    pub resolved: Value,
    pub provider_options: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedCanonicalField {
    pub quality: CanonicalMatchQuality,
    pub resolution: Option<CanonicalResolution>,
    pub omit: bool,
}

impl ResolvedCanonicalField {
    pub(crate) fn satisfies(&self, requirement: &CanonicalFieldRequirement) -> bool {
        if !requirement.strict {
            return self.quality != CanonicalMatchQuality::Unsupported;
        }
        matches!(
            self.quality,
            CanonicalMatchQuality::Exact | CanonicalMatchQuality::Fuzzy
        )
    }
}

pub(crate) fn resolve_canonical_field(
    mapping: Option<&CanonicalFieldMapping>,
    requirement: &CanonicalFieldRequirement,
) -> ResolvedCanonicalField {
    let Some(mapping) = mapping else {
        return ResolvedCanonicalField {
            quality: CanonicalMatchQuality::Default,
            resolution: None,
            omit: true,
        };
    };
    let converted = convert_canonical_field(&mapping.converter, requirement);
    if converted.quality != CanonicalMatchQuality::Unsupported {
        return converted;
    }
    if requirement.strict {
        return unsupported();
    }
    apply_fallback(&mapping.converter, &mapping.fallback)
}

pub(crate) fn resolve_missing_canonical_field(
    mapping: &CanonicalFieldMapping,
) -> ResolvedCanonicalField {
    apply_fallback(&mapping.converter, &mapping.fallback)
}

fn convert_canonical_field(
    resolver: &CanonicalFieldConverter,
    requirement: &CanonicalFieldRequirement,
) -> ResolvedCanonicalField {
    match resolver {
        CanonicalFieldConverter::Passthrough => {
            resolved(CanonicalMatchQuality::Exact, requirement.value.clone())
        }
        CanonicalFieldConverter::Prompt => ResolvedCanonicalField {
            quality: CanonicalMatchQuality::Prompt,
            resolution: None,
            omit: false,
        },
        CanonicalFieldConverter::OpenaiTtsVoiceV1 => resolve_openai_tts_voice_v1(requirement),
        CanonicalFieldConverter::OpenaiTtsVoiceV2 => resolve_openai_tts_voice_v2(requirement),
        CanonicalFieldConverter::GeminiTtsVoiceV1 => resolve_gemini_voice(requirement),
        CanonicalFieldConverter::GlmTtsVoiceV1 => resolve_glm_voice(requirement),
        CanonicalFieldConverter::MinimaxTtsVoiceV1 => resolve_minimax_voice(requirement),
    }
}

fn apply_fallback(
    resolver: &CanonicalFieldConverter,
    fallback: &CanonicalFallback,
) -> ResolvedCanonicalField {
    match fallback {
        CanonicalFallback::Reject => unsupported(),
        CanonicalFallback::Omit => ResolvedCanonicalField {
            quality: CanonicalMatchQuality::Default,
            resolution: None,
            omit: true,
        },
        CanonicalFallback::Default { value } => {
            let requirement = CanonicalFieldRequirement::new(value.clone());
            let mut result = convert_canonical_field(resolver, &requirement);
            if result.quality == CanonicalMatchQuality::Unsupported || result.resolution.is_none() {
                return unsupported();
            }
            result.quality = CanonicalMatchQuality::Default;
            result
        }
    }
}

fn resolve_openai_tts_voice_v1(requirement: &CanonicalFieldRequirement) -> ResolvedCanonicalField {
    resolve_openai_voice(requirement, false)
}

fn resolve_openai_tts_voice_v2(requirement: &CanonicalFieldRequirement) -> ResolvedCanonicalField {
    resolve_openai_voice(requirement, true)
}

fn resolve_openai_voice(
    requirement: &CanonicalFieldRequirement,
    supports_instructions: bool,
) -> ResolvedCanonicalField {
    let Some(requested) = parse_voice_spec(requirement) else {
        return unsupported();
    };
    if !supports_instructions && requested.instructions.is_some() {
        return unsupported();
    }
    let has_voice_constraints = requested.gender.is_some() || requested.style.is_some();
    let voice = if has_voice_constraints {
        if !requirement.allow_fuzzy {
            return unsupported();
        }
        let Some(voice) = select_openai_voice(&requested, supports_instructions) else {
            return unsupported();
        };
        voice
    } else {
        "alloy"
    };
    let mut options = BTreeMap::from([("voice".to_owned(), json!(voice))]);
    if supports_instructions {
        if let Some(value) = combine_voice_instructions(&requested) {
            options.insert("instructions".to_owned(), Value::String(value));
        }
    }
    let quality = if has_voice_constraints {
        CanonicalMatchQuality::Fuzzy
    } else if supports_instructions && requested.instructions.is_some() {
        CanonicalMatchQuality::Prompt
    } else {
        CanonicalMatchQuality::Exact
    };
    resolved_with_options(quality, requirement.value.clone(), options)
}

struct OpenAiVoiceProfile {
    voice: &'static str,
    gender: VoiceGender,
    styles: &'static [VoiceStyle],
    legacy: bool,
}

const OPENAI_VOICES: &[OpenAiVoiceProfile] = &[
    OpenAiVoiceProfile {
        voice: "alloy",
        gender: VoiceGender::Neutral,
        styles: &[VoiceStyle::Even, VoiceStyle::Clear],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "ash",
        gender: VoiceGender::Male,
        styles: &[VoiceStyle::Casual, VoiceStyle::Lively],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "ballad",
        gender: VoiceGender::Male,
        styles: &[VoiceStyle::Warm, VoiceStyle::Gentle],
        legacy: false,
    },
    OpenAiVoiceProfile {
        voice: "coral",
        gender: VoiceGender::Female,
        styles: &[VoiceStyle::Friendly, VoiceStyle::Clear],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "echo",
        gender: VoiceGender::Male,
        styles: &[VoiceStyle::Smooth, VoiceStyle::Casual],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "fable",
        gender: VoiceGender::Neutral,
        styles: &[VoiceStyle::Lively, VoiceStyle::Warm],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "nova",
        gender: VoiceGender::Female,
        styles: &[VoiceStyle::Upbeat, VoiceStyle::Friendly],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "onyx",
        gender: VoiceGender::Male,
        styles: &[VoiceStyle::Mature, VoiceStyle::Firm],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "sage",
        gender: VoiceGender::Female,
        styles: &[VoiceStyle::Knowledgeable, VoiceStyle::Even],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "shimmer",
        gender: VoiceGender::Female,
        styles: &[VoiceStyle::Bright, VoiceStyle::Soft],
        legacy: true,
    },
    OpenAiVoiceProfile {
        voice: "verse",
        gender: VoiceGender::Male,
        styles: &[VoiceStyle::Casual, VoiceStyle::Lively],
        legacy: false,
    },
    OpenAiVoiceProfile {
        voice: "marin",
        gender: VoiceGender::Female,
        styles: &[VoiceStyle::Warm, VoiceStyle::Clear],
        legacy: false,
    },
    OpenAiVoiceProfile {
        voice: "cedar",
        gender: VoiceGender::Male,
        styles: &[VoiceStyle::Mature, VoiceStyle::Clear],
        legacy: false,
    },
];

fn select_openai_voice(requested: &VoiceSpec, supports_all_voices: bool) -> Option<&'static str> {
    OPENAI_VOICES
        .iter()
        .filter(|profile| supports_all_voices || profile.legacy)
        .filter_map(|profile| {
            let gender_score = requested
                .gender
                .is_some_and(|gender| gender == profile.gender)
                as u8;
            let style_score = requested
                .style
                .is_some_and(|style| profile.styles.contains(&style))
                as u8;
            let score = gender_score + style_score * 2;
            (score > 0).then_some((score, profile.voice))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, voice)| voice)
}

const GEMINI_VOICES: &[(&str, VoiceStyle)] = &[
    ("Zephyr", VoiceStyle::Bright),
    ("Puck", VoiceStyle::Upbeat),
    ("Charon", VoiceStyle::Informative),
    ("Kore", VoiceStyle::Firm),
    ("Fenrir", VoiceStyle::Excitable),
    ("Leda", VoiceStyle::Youthful),
    ("Orus", VoiceStyle::Firm),
    ("Aoede", VoiceStyle::Breezy),
    ("Callirrhoe", VoiceStyle::EasyGoing),
    ("Autonoe", VoiceStyle::Bright),
    ("Enceladus", VoiceStyle::Breathy),
    ("Iapetus", VoiceStyle::Clear),
    ("Umbriel", VoiceStyle::EasyGoing),
    ("Algieba", VoiceStyle::Smooth),
    ("Despina", VoiceStyle::Smooth),
    ("Erinome", VoiceStyle::Clear),
    ("Algenib", VoiceStyle::Gravelly),
    ("Rasalgethi", VoiceStyle::Informative),
    ("Laomedeia", VoiceStyle::Upbeat),
    ("Achernar", VoiceStyle::Soft),
    ("Alnilam", VoiceStyle::Firm),
    ("Schedar", VoiceStyle::Even),
    ("Gacrux", VoiceStyle::Mature),
    ("Pulcherrima", VoiceStyle::Forward),
    ("Achird", VoiceStyle::Friendly),
    ("Zubenelgenubi", VoiceStyle::Casual),
    ("Vindemiatrix", VoiceStyle::Gentle),
    ("Sadachbia", VoiceStyle::Lively),
    ("Sadaltager", VoiceStyle::Knowledgeable),
    ("Sulafat", VoiceStyle::Warm),
];

fn resolve_gemini_voice(requirement: &CanonicalFieldRequirement) -> ResolvedCanonicalField {
    let Some(requested) = parse_voice_spec(requirement) else {
        return unsupported();
    };
    if requested.gender.is_some() || requested.instructions.is_some() {
        return unsupported();
    }
    let style = requested.style;
    let (voice, quality) = if let Some(style) = style {
        if !requirement.allow_fuzzy {
            return unsupported();
        }
        let mut voices = GEMINI_VOICES
            .iter()
            .filter(|(_, candidate_style)| *candidate_style == style)
            .map(|(voice, _)| *voice)
            .collect::<Vec<_>>();
        voices.sort_unstable();
        let Some(voice) = voices.first() else {
            return unsupported();
        };
        (*voice, CanonicalMatchQuality::Fuzzy)
    } else {
        ("Puck", CanonicalMatchQuality::Exact)
    };
    let mut speech = Map::from_iter([("voice".to_owned(), json!(voice))]);
    if let Some(language) = &requested.language {
        speech.insert("language".to_owned(), json!(language));
    }
    let options = BTreeMap::from([(
        "generation_config".to_owned(),
        json!({"speech_config": [speech]}),
    )]);
    resolved_with_options(quality, requirement.value.clone(), options)
}

fn resolve_minimax_voice(requirement: &CanonicalFieldRequirement) -> ResolvedCanonicalField {
    let Some(requested) = parse_voice_spec(requirement) else {
        return unsupported();
    };
    if requested == VoiceSpec::default() {
        return resolved_with_options(
            CanonicalMatchQuality::Exact,
            requirement.value.clone(),
            BTreeMap::from([(
                "voice_setting".to_owned(),
                json!({"voice_id": "male-qn-qingse"}),
            )]),
        );
    }
    unsupported()
}

struct GlmVoiceProfile {
    voice: &'static str,
    gender: VoiceGender,
    styles: &'static [VoiceStyle],
}

const GLM_VOICES: &[GlmVoiceProfile] = &[
    GlmVoiceProfile {
        voice: "tongtong",
        gender: VoiceGender::Female,
        styles: &[
            VoiceStyle::Warm,
            VoiceStyle::Friendly,
            VoiceStyle::Gentle,
            VoiceStyle::Clear,
            VoiceStyle::Soft,
            VoiceStyle::Even,
        ],
    },
    GlmVoiceProfile {
        voice: "xiaochen",
        gender: VoiceGender::Male,
        styles: &[
            VoiceStyle::Informative,
            VoiceStyle::Knowledgeable,
            VoiceStyle::Mature,
            VoiceStyle::Firm,
            VoiceStyle::Clear,
            VoiceStyle::Even,
        ],
    },
    GlmVoiceProfile {
        voice: "chuichui",
        gender: VoiceGender::Female,
        styles: &[
            VoiceStyle::Bright,
            VoiceStyle::Upbeat,
            VoiceStyle::Youthful,
            VoiceStyle::Lively,
            VoiceStyle::Excitable,
        ],
    },
    GlmVoiceProfile {
        voice: "jam",
        gender: VoiceGender::Neutral,
        styles: &[VoiceStyle::Casual, VoiceStyle::Lively, VoiceStyle::Breezy],
    },
    GlmVoiceProfile {
        voice: "kazi",
        gender: VoiceGender::Neutral,
        styles: &[VoiceStyle::Gravelly, VoiceStyle::Forward, VoiceStyle::Firm],
    },
    GlmVoiceProfile {
        voice: "douji",
        gender: VoiceGender::Neutral,
        styles: &[
            VoiceStyle::Excitable,
            VoiceStyle::Upbeat,
            VoiceStyle::Casual,
        ],
    },
    GlmVoiceProfile {
        voice: "luodo",
        gender: VoiceGender::Neutral,
        styles: &[
            VoiceStyle::Smooth,
            VoiceStyle::EasyGoing,
            VoiceStyle::Breathy,
        ],
    },
];

fn resolve_glm_voice(requirement: &CanonicalFieldRequirement) -> ResolvedCanonicalField {
    let Some(requested) = parse_voice_spec(requirement) else {
        return unsupported();
    };
    if requested.instructions.is_some() {
        return unsupported();
    }
    if let Some(language) = &requested.language {
        let normalized = language.to_ascii_lowercase().replace('_', "-");
        if normalized != "zh"
            && normalized != "zh-cn"
            && normalized != "cmn"
            && normalized != "cmn-cn"
        {
            return unsupported();
        }
    }
    let has_voice_constraints = requested.gender.is_some() || requested.style.is_some();
    let (voice, quality) = if has_voice_constraints {
        if !requirement.allow_fuzzy {
            return unsupported();
        }
        let Some(voice) = select_glm_voice(&requested) else {
            return unsupported();
        };
        (voice, CanonicalMatchQuality::Fuzzy)
    } else {
        ("tongtong", CanonicalMatchQuality::Exact)
    };
    resolved_with_options(
        quality,
        requirement.value.clone(),
        BTreeMap::from([("voice".to_owned(), json!(voice))]),
    )
}

fn select_glm_voice(requested: &VoiceSpec) -> Option<&'static str> {
    GLM_VOICES
        .iter()
        .filter_map(|profile| {
            let gender_score = requested
                .gender
                .is_some_and(|gender| gender == profile.gender)
                as u8;
            let style_score = requested
                .style
                .is_some_and(|style| profile.styles.contains(&style))
                as u8;
            let score = gender_score + style_score * 2;
            (score > 0).then_some((score, profile.voice))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, voice)| voice)
}

fn combine_voice_instructions(requested: &VoiceSpec) -> Option<String> {
    let instructions = requested.instructions.as_deref();
    let style = requested
        .style
        .and_then(|style| serde_json::to_value(style).ok())
        .and_then(|style| style.as_str().map(str::to_owned));
    match (instructions, style.as_deref()) {
        (Some(instructions), Some(style)) => Some(format!("{instructions}\nStyle: {style}")),
        (Some(instructions), None) => Some(instructions.to_owned()),
        (None, Some(style)) => Some(format!("Style: {style}")),
        (None, None) => None,
    }
}

fn parse_voice_spec(requirement: &CanonicalFieldRequirement) -> Option<VoiceSpec> {
    serde_json::from_value(requirement.value.clone()).ok()
}

fn resolved(quality: CanonicalMatchQuality, value: Value) -> ResolvedCanonicalField {
    resolved_with_options(quality, value, BTreeMap::new())
}

fn resolved_with_options(
    quality: CanonicalMatchQuality,
    value: Value,
    provider_options: BTreeMap<String, Value>,
) -> ResolvedCanonicalField {
    ResolvedCanonicalField {
        quality,
        resolution: Some(CanonicalResolution {
            resolved: value,
            provider_options,
        }),
        omit: false,
    }
}

fn unsupported() -> ResolvedCanonicalField {
    ResolvedCanonicalField {
        quality: CanonicalMatchQuality::Unsupported,
        resolution: None,
        omit: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_resolvers_cover_exact_default_and_strict_results() {
        let openai = CanonicalFieldMapping {
            converter: CanonicalFieldConverter::OpenaiTtsVoiceV2,
            fallback: CanonicalFallback::Default { value: json!({}) },
        };
        let exact = CanonicalFieldRequirement::new(json!({}));
        assert_eq!(
            resolve_canonical_field(Some(&openai), &exact).quality,
            CanonicalMatchQuality::Exact
        );
        let directed = CanonicalFieldRequirement::strict(json!({
            "instructions": "Speak calmly"
        }));
        let directed_result = resolve_canonical_field(Some(&openai), &directed);
        assert_eq!(directed_result.quality, CanonicalMatchQuality::Prompt);
        assert!(!directed_result.satisfies(&directed));
        assert_eq!(
            directed_result.resolution.unwrap().provider_options["instructions"],
            "Speak calmly"
        );
        let openai_styled = resolve_canonical_field(
            Some(&openai),
            &CanonicalFieldRequirement::new(json!({
                "gender": "female",
                "style": "warm"
            })),
        );
        assert_eq!(openai_styled.quality, CanonicalMatchQuality::Fuzzy);
        assert_eq!(
            openai_styled.resolution.unwrap().provider_options["voice"],
            "marin"
        );
        let styled = CanonicalFieldRequirement::new(json!({"style": "warm"}));
        let gemini = CanonicalFieldMapping::required(CanonicalFieldConverter::GeminiTtsVoiceV1);
        let styled_result = resolve_canonical_field(Some(&gemini), &styled);
        assert_eq!(styled_result.quality, CanonicalMatchQuality::Fuzzy);
        assert_eq!(
            styled_result.resolution.unwrap().provider_options["generation_config"]
                ["speech_config"][0]["voice"],
            "Sulafat"
        );
        let no_fuzzy = CanonicalFieldRequirement::strict(json!({"style": "warm"}));
        assert_eq!(
            resolve_canonical_field(Some(&gemini), &no_fuzzy).quality,
            CanonicalMatchQuality::Unsupported
        );
        let unmapped = CanonicalFieldRequirement::new(json!({"style": "warm"}));
        let unmapped_result = resolve_canonical_field(None, &unmapped);
        assert!(unmapped_result.omit);
        assert!(unmapped_result.satisfies(&unmapped));
        let strict_unmapped = CanonicalFieldRequirement::strict(json!({"style": "warm"}));
        assert!(!resolve_canonical_field(None, &strict_unmapped).satisfies(&strict_unmapped));
        let strict_unsupported = CanonicalFieldRequirement::strict(json!({"unsupported": "value"}));
        let defaulted = resolve_canonical_field(Some(&openai), &strict_unsupported);
        assert_eq!(defaulted.quality, CanonicalMatchQuality::Unsupported);
        assert!(!defaulted.satisfies(&strict_unsupported));
        let legacy = CanonicalFieldMapping {
            converter: CanonicalFieldConverter::OpenaiTtsVoiceV1,
            fallback: CanonicalFallback::Default { value: json!({}) },
        };
        assert_eq!(
            resolve_canonical_field(
                Some(&legacy),
                &CanonicalFieldRequirement::strict(json!({"style": "warm"}))
            )
            .quality,
            CanonicalMatchQuality::Unsupported
        );
        assert_eq!(
            resolve_canonical_field(
                Some(&legacy),
                &CanonicalFieldRequirement::new(json!({"style": "warm"}))
            )
            .quality,
            CanonicalMatchQuality::Fuzzy
        );
        assert_eq!(
            resolve_canonical_field(
                Some(&legacy),
                &CanonicalFieldRequirement::new(json!({"style": "warm"}))
            )
            .resolution
            .unwrap()
            .provider_options["voice"],
            "fable"
        );
        let glm = CanonicalFieldMapping {
            converter: CanonicalFieldConverter::GlmTtsVoiceV1,
            fallback: CanonicalFallback::Default { value: json!({}) },
        };
        let glm_zh = resolve_canonical_field(
            Some(&glm),
            &CanonicalFieldRequirement::new(json!({"language": "zh-CN"})),
        );
        assert_eq!(glm_zh.quality, CanonicalMatchQuality::Exact);
        assert_eq!(
            glm_zh.resolution.unwrap().provider_options["voice"],
            "tongtong"
        );
        let glm_male = resolve_canonical_field(
            Some(&glm),
            &CanonicalFieldRequirement::new(json!({"gender": "male"})),
        );
        assert_eq!(glm_male.quality, CanonicalMatchQuality::Fuzzy);
        assert_eq!(
            glm_male.resolution.unwrap().provider_options["voice"],
            "xiaochen"
        );
        let glm_warm = resolve_canonical_field(
            Some(&glm),
            &CanonicalFieldRequirement::new(json!({"style": "warm"})),
        );
        assert_eq!(glm_warm.quality, CanonicalMatchQuality::Fuzzy);
        assert_eq!(
            glm_warm.resolution.unwrap().provider_options["voice"],
            "tongtong"
        );
        let glm_gravelly = resolve_canonical_field(
            Some(&glm),
            &CanonicalFieldRequirement::new(json!({"style": "gravelly"})),
        );
        assert_eq!(glm_gravelly.quality, CanonicalMatchQuality::Fuzzy);
        assert_eq!(
            glm_gravelly.resolution.unwrap().provider_options["voice"],
            "kazi"
        );
        let glm_easy_going = resolve_canonical_field(
            Some(&glm),
            &CanonicalFieldRequirement::new(json!({"style": "easy_going"})),
        );
        assert_eq!(glm_easy_going.quality, CanonicalMatchQuality::Fuzzy);
        assert_eq!(
            glm_easy_going.resolution.unwrap().provider_options["voice"],
            "luodo"
        );
        assert_eq!(
            resolve_canonical_field(
                Some(&glm),
                &CanonicalFieldRequirement::strict(json!({"style": "warm"}))
            )
            .quality,
            CanonicalMatchQuality::Unsupported
        );
        assert_eq!(
            resolve_missing_canonical_field(&openai)
                .resolution
                .unwrap()
                .provider_options["voice"],
            "alloy"
        );
    }

    #[test]
    fn converter_names_are_the_metadata_contract() {
        assert_eq!(
            serde_json::to_value(CanonicalFieldConverter::GeminiTtsVoiceV1).unwrap(),
            json!("gemini_tts_voice_v1")
        );
        assert_eq!(
            serde_json::to_value(CanonicalFieldConverter::GlmTtsVoiceV1).unwrap(),
            json!("glm_tts_voice_v1")
        );
        assert!(serde_json::from_value::<CanonicalFieldConverter>(json!("unknown")).is_err());
        let mapping: CanonicalFieldMapping = serde_json::from_value(json!({
            "converter": "openai_tts_voice_v1",
            "fallback": {"action": "omit"}
        }))
        .unwrap();
        assert_eq!(mapping.fallback, CanonicalFallback::Omit);
        assert!(mapping.validate().is_ok());
    }

    #[test]
    fn fallback_policy_handles_absent_unconvertible_and_invalid_defaults() {
        let optional = CanonicalFieldMapping {
            converter: CanonicalFieldConverter::OpenaiTtsVoiceV1,
            fallback: CanonicalFallback::Omit,
        };
        assert!(resolve_missing_canonical_field(&optional).omit);
        assert!(
            resolve_canonical_field(
                Some(&optional),
                &CanonicalFieldRequirement::new(json!({"instructions": "calm"}))
            )
            .omit
        );

        let invalid_default = CanonicalFieldMapping {
            converter: CanonicalFieldConverter::OpenaiTtsVoiceV1,
            fallback: CanonicalFallback::Default {
                value: json!({"style": "unknown"}),
            },
        };
        assert!(invalid_default.validate().is_err());
    }
}
