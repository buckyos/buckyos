//! Credit price list (doc/aicc/aicc_user_policy.md §3): prices by Provider
//! independent model identity or exact model, administrator Credit rates of
//! Provider cost currencies, and the Credit amount of one usage.

use std::collections::BTreeMap;

use buckyos_api::{
    AiCost, AiUsage, CreditChargeStatus, CreditPriceSource, CreditPriceState, ModelCreditPrice,
    PriceEntry, CREDIT_MICRO_PER_UNIT,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Inventory, InventoryModel};
use crate::catalog::Pricing;
use crate::execution::PinnedPricingSnapshot;

pub(crate) const CREDIT_CURRENCY: &str = "CREDIT";
pub(crate) const MODEL_KEY_PREFIX: &str = "model:";
pub(crate) const EXACT_KEY_PREFIX: &str = "exact:";
/// Token rates are entered per million tokens and stored per token.
const TOKENS_PER_RATE_UNIT: f64 = 1_000_000.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EntryKey {
    Model {
        model_driver_id: String,
        origin_model_id: String,
    },
    Exact {
        provider_model_id: String,
        variant: Option<String>,
        provider_instance_name: String,
    },
}

pub(crate) fn parse_entry_key(key: &str) -> Result<EntryKey, &'static str> {
    if let Some(rest) = key.strip_prefix(MODEL_KEY_PREFIX) {
        let (driver, origin) = rest
            .split_once(':')
            .ok_or("a model price key is model:<model_driver_id>:<origin_model_id>")?;
        if driver.trim().is_empty() || origin.trim().is_empty() {
            return Err("a model price key is model:<model_driver_id>:<origin_model_id>");
        }
        return Ok(EntryKey::Model {
            model_driver_id: driver.to_string(),
            origin_model_id: origin.to_string(),
        });
    }
    if let Some(rest) = key.strip_prefix(EXACT_KEY_PREFIX) {
        let exact = crate::model::ExactModelName::parse(rest)
            .map_err(|_| "an exact price key is exact:<provider_model_id>[:variant]@<instance>")?;
        return Ok(EntryKey::Exact {
            provider_model_id: exact.provider_model_id().to_string(),
            variant: exact.variant().map(str::to_owned),
            provider_instance_name: exact.provider_instance_name().to_string(),
        });
    }
    Err("a price key starts with model: or exact:")
}

/// Scale every `*_token` rate of a pricing value (`factor` 1e-6 turns per
/// million rates into per token ones).
pub(crate) fn scale_token_rates(value: &mut Value, factor: f64) {
    match value {
        Value::Object(object) => {
            for (key, item) in object.iter_mut() {
                if key == "when" {
                    continue;
                }
                match item {
                    Value::Number(number) if key.ends_with("_token") => {
                        if let Some(rate) = number.as_f64() {
                            *item = serde_json::Number::from_f64(rate * factor)
                                .map(Value::Number)
                                .unwrap_or(Value::Null);
                        }
                    }
                    _ => scale_token_rates(item, factor),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| scale_token_rates(item, factor)),
        _ => {}
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredPrice {
    pub entry_key: String,
    /// Credit pricing with token rates per token.
    pub pricing: Pricing,
    pub note: Option<String>,
    pub updated_by: String,
    pub updated_at_ms: i64,
}

impl StoredPrice {
    /// API form: token rates per million tokens.
    pub(crate) fn to_entry(&self) -> PriceEntry {
        let mut pricing = serde_json::to_value(&self.pricing).unwrap_or(Value::Null);
        scale_token_rates(&mut pricing, TOKENS_PER_RATE_UNIT);
        PriceEntry {
            entry_key: self.entry_key.clone(),
            pricing,
            note: self.note.clone(),
            updated_by: self.updated_by.clone(),
            updated_at_ms: self.updated_at_ms,
        }
    }
}

/// Parse an API price (rates per million tokens) into stored Credit pricing.
pub(crate) fn pricing_from_api(value: &Value) -> Result<Pricing, String> {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object
            .entry("currency")
            .or_insert_with(|| Value::String(CREDIT_CURRENCY.to_string()));
    }
    scale_token_rates(&mut value, 1.0 / TOKENS_PER_RATE_UNIT);
    let pricing: Pricing =
        serde_json::from_value(value).map_err(|error| format!("invalid pricing: {error}"))?;
    if pricing.currency.trim() != CREDIT_CURRENCY {
        return Err("a price list entry is priced in CREDIT".to_string());
    }
    if pricing.estimated_cost.is_some() {
        return Err("a price list entry has no estimated cost".to_string());
    }
    crate::catalog::validate_pricing("price list", &pricing).map_err(|error| error.to_string())?;
    match PinnedPricingSnapshot::from_pricing(&pricing, None, std::time::SystemTime::now()) {
        Ok(Some(_)) => Ok(pricing),
        Ok(None) if pricing.unit.is_some() && !pricing.rules.is_empty() => Ok(pricing),
        Ok(None) => Err("a price list entry needs token rates or a unit price".to_string()),
        Err(error) => Err(error.message),
    }
}

/// True when every rate of the price is zero: an explicitly free model.
pub(crate) fn pricing_is_zero(pricing: &Pricing) -> bool {
    let zero = |value: Option<f64>| value.is_none_or(|value| value == 0.0);
    let token_rates = [
        pricing.input_token,
        pricing.output_token,
        pricing.cache_input_token,
        pricing.cache_write_input_token,
        pricing.cache_write_1h_input_token,
        pricing.audio_input_token,
        pricing.image_input_token,
        pricing.audio_output_token,
        pricing.image_output_token,
        pricing.amount,
    ];
    token_rates.into_iter().all(zero)
        && pricing.rules.iter().all(|rule| rule.amount == 0.0)
        && pricing.addons.iter().all(|addon| addon.amount == 0.0)
        && pricing.tiers.iter().flat_map(|tiers| &tiers.steps).all(|step| {
            [
                step.input_token,
                step.output_token,
                step.cache_input_token,
                step.cache_write_input_token,
                step.cache_write_1h_input_token,
                step.audio_input_token,
                step.image_input_token,
                step.audio_output_token,
                step.image_output_token,
                step.amount,
            ]
            .into_iter()
            .all(zero)
        })
        && pricing.time_windows.iter().all(|window| {
            [
                window.input_token,
                window.output_token,
                window.cache_input_token,
                window.cache_write_input_token,
                window.cache_write_1h_input_token,
                window.audio_input_token,
                window.image_input_token,
                window.audio_output_token,
                window.image_output_token,
                window.amount,
            ]
            .into_iter()
            .all(zero)
        })
}

/// The Zone's price list.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PriceList {
    pub revision: u64,
    pub entries: BTreeMap<String, StoredPrice>,
    /// Currency → Credit per unit of Provider cost.
    pub credit_rates: BTreeMap<String, f64>,
}

/// Where the price of a model comes from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UnitPrice<'a> {
    Credit {
        source: CreditPriceSource,
        entry: &'a StoredPrice,
    },
    ProviderCost {
        currency: String,
        rate: f64,
    },
    None,
}

impl PriceList {
    /// `exact:` (with the variant, then without), then `model:` for a model
    /// mapped to a Model Driver.
    pub(crate) fn entry_for(&self, model: &InventoryModel) -> Option<(CreditPriceSource, &StoredPrice)> {
        let instance = &model.provider_instance_name;
        let exact_keys = [
            Some(format!("{EXACT_KEY_PREFIX}{}", model.exact_model)),
            model.variant.as_ref().map(|_| {
                format!("{EXACT_KEY_PREFIX}{}@{instance}", model.provider_model_id)
            }),
        ];
        if let Some(entry) = exact_keys
            .into_iter()
            .flatten()
            .find_map(|key| self.entries.get(&key))
        {
            return Some((CreditPriceSource::Exact, entry));
        }
        if model.model_driver_id.is_empty() || model.origin_model_id.is_empty() {
            return None;
        }
        self.entries
            .get(&format!(
                "{MODEL_KEY_PREFIX}{}:{}",
                model.model_driver_id, model.origin_model_id
            ))
            .map(|entry| (CreditPriceSource::Model, entry))
    }

    pub(crate) fn rate(&self, currency: &str) -> Option<f64> {
        self.credit_rates
            .get(&currency.trim().to_ascii_uppercase())
            .copied()
            .filter(|rate| rate.is_finite() && *rate >= 0.0)
    }

    /// Unit price of a model before a call; the Provider cost path uses the
    /// inventory's knowledge of its cost currency.
    pub(crate) fn unit_price<'a>(&'a self, model: &InventoryModel) -> UnitPrice<'a> {
        if let Some((source, entry)) = self.entry_for(model) {
            return UnitPrice::Credit { source, entry };
        }
        match &model.provider_cost {
            Some(cost) => match self.rate(&cost.currency) {
                Some(rate) => UnitPrice::ProviderCost {
                    currency: cost.currency.clone(),
                    rate,
                },
                None => UnitPrice::None,
            },
            None => UnitPrice::None,
        }
    }

    pub(crate) fn state(&self, model: &InventoryModel, coefficient: f64) -> CreditPriceState {
        if coefficient == 0.0 {
            return CreditPriceState::Free;
        }
        match self.unit_price(model) {
            UnitPrice::Credit { entry, .. } if pricing_is_zero(&entry.pricing) => {
                CreditPriceState::Free
            }
            UnitPrice::ProviderCost { rate, .. }
                if rate == 0.0 || model.provider_cost.as_ref().is_some_and(|cost| cost.zero) =>
            {
                CreditPriceState::Free
            }
            UnitPrice::Credit { .. } | UnitPrice::ProviderCost { .. } => CreditPriceState::Priced,
            UnitPrice::None => CreditPriceState::Unpriced,
        }
    }

    pub(crate) fn describe(
        &self,
        model: &InventoryModel,
        provider_coefficient: f64,
        model_coefficient: f64,
    ) -> ModelCreditPrice {
        let state = self.state(model, provider_coefficient * model_coefficient);
        let mut price = ModelCreditPrice {
            state,
            source: None,
            entry_key: None,
            pricing: None,
            provider_cost_currency: None,
            credit_rate: None,
            provider_coefficient,
            model_coefficient,
        };
        match self.unit_price(model) {
            UnitPrice::Credit { source, entry } => {
                price.source = Some(source);
                price.entry_key = Some(entry.entry_key.clone());
                price.pricing = Some(entry.to_entry().pricing);
            }
            UnitPrice::ProviderCost { currency, rate } => {
                price.source = Some(CreditPriceSource::ProviderCost);
                price.provider_cost_currency = Some(currency);
                price.credit_rate = Some(rate);
            }
            UnitPrice::None => {}
        }
        price
    }

    /// Inventory models with no price at all (ignoring coefficients).
    pub(crate) fn unpriced(&self, inventory: &Inventory) -> Vec<String> {
        inventory
            .models()
            .filter(|model| matches!(self.unit_price(model), UnitPrice::None))
            .map(|model| model.exact_model.clone())
            .collect()
    }
}

/// The Credit price pinned for an admitted attempt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PinnedCreditKind {
    /// Explicitly free: nothing is charged whatever the usage.
    Free,
    Credit {
        entry_key: String,
        snapshot: PinnedPricingSnapshot,
    },
    ProviderCost {
        currency: String,
        rate: f64,
    },
    /// Only admitted with a zero coefficient; kept for the ledger.
    Unpriced,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PinnedCreditPrice {
    #[serde(flatten)]
    pub kind: PinnedCreditKind,
    pub provider_coefficient: f64,
    pub model_coefficient: f64,
}

impl PinnedCreditPrice {
    fn coefficient(&self) -> f64 {
        self.provider_coefficient * self.model_coefficient
    }

    /// Whether the balance gate applies: the call may cost something.
    pub(crate) fn needs_payment(&self) -> bool {
        self.coefficient() > 0.0
            && match &self.kind {
                PinnedCreditKind::Credit { .. } => true,
                PinnedCreditKind::ProviderCost { rate, .. } => *rate > 0.0,
                PinnedCreditKind::Free | PinnedCreditKind::Unpriced => false,
            }
    }

    /// Credit of one usage, rounded to 1e-6, and how it was charged.
    pub(crate) fn charge(
        &self,
        usage: &AiUsage,
        provider_cost: Option<&AiCost>,
    ) -> (i64, CreditChargeStatus) {
        if self.coefficient() == 0.0 {
            return (0, CreditChargeStatus::Free);
        }
        let credit = match &self.kind {
            PinnedCreditKind::Free => return (0, CreditChargeStatus::Free),
            PinnedCreditKind::Unpriced => None,
            PinnedCreditKind::Credit { snapshot, .. } => {
                snapshot.completion_cost(usage).map(|cost| cost.amount)
            }
            PinnedCreditKind::ProviderCost { currency, rate } => provider_cost
                .filter(|cost| cost.currency.trim().eq_ignore_ascii_case(currency))
                .map(|cost| cost.amount * rate),
        };
        match credit.map(|credit| credit * self.coefficient()) {
            Some(credit) if credit.is_finite() && credit >= 0.0 => {
                let micro = (credit * CREDIT_MICRO_PER_UNIT as f64).round();
                if micro > i64::MAX as f64 {
                    (0, CreditChargeStatus::Unpriced)
                } else {
                    (micro as i64, CreditChargeStatus::Charged)
                }
            }
            _ => (0, CreditChargeStatus::Unpriced),
        }
    }
}
