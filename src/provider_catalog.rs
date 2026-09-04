//! Asset-free metadata for audited local provider imports.

use serde::Deserialize;
use stack_theme::{
    ProviderNodeKind, ProviderPackAdditionalSource, ProviderPackIdentity, ProviderPackNotice,
    ProviderPackRights, ProviderPackSource,
};

const AWS_CATALOG: &str = include_str!("../catalogs/aws.json");
const GCP_CATALOG: &str = include_str!("../catalogs/gcp.json");
const AZURE_CATALOG: &str = include_str!("../catalogs/azure.json");
const SIMPLE_ICONS_CATALOG: &str = include_str!("../catalogs/simple-icons.json");

pub(crate) const PROVIDER_IDS: [&str; 4] = ["aws", "gcp", "azure", "simple-icons"];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderCatalog {
    pub(crate) catalog_version: String,
    pub(crate) pack_version: String,
    pub(crate) provider: ProviderPackIdentity,
    pub(crate) source: ProviderPackSource,
    #[serde(default)]
    pub(crate) additional_sources: Vec<ProviderPackAdditionalSource>,
    pub(crate) rights: ProviderPackRights,
    pub(crate) notice: ProviderPackNotice,
    pub(crate) icons: Vec<CatalogIcon>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CatalogIcon {
    pub(crate) id: String,
    pub(crate) subject: String,
    pub(crate) product_name: String,
    #[serde(default)]
    pub(crate) brand_source_url: Option<String>,
    #[serde(default)]
    pub(crate) brand_guidelines_url: Option<String>,
    pub(crate) recommended_node_kind: ProviderNodeKind,
    pub(crate) category: String,
    #[serde(default)]
    pub(crate) source_id: Option<String>,
    pub(crate) archive_path: String,
}

pub(crate) fn provider_catalog(provider: &str) -> Result<ProviderCatalog, String> {
    let source = match provider {
        "aws" => AWS_CATALOG,
        "gcp" => GCP_CATALOG,
        "azure" => AZURE_CATALOG,
        "simple-icons" => SIMPLE_ICONS_CATALOG,
        _ => {
            return Err(format!(
                "unknown provider '{provider}'; expected aws, gcp, azure, or simple-icons"
            ));
        }
    };
    let catalog: ProviderCatalog = serde_json::from_str(source)
        .map_err(|_| format!("embedded provider catalog '{provider}' is invalid"))?;
    if catalog.catalog_version != "1.0" {
        return Err(format!(
            "embedded provider catalog '{provider}' uses an unsupported version"
        ));
    }
    Ok(catalog)
}

pub(crate) fn provider_catalogs() -> Result<Vec<ProviderCatalog>, String> {
    PROVIDER_IDS.into_iter().map(provider_catalog).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn embedded_catalogs_have_expected_complete_unique_coverage() {
        let catalogs = provider_catalogs();
        assert!(catalogs.is_ok());
        let Ok(catalogs) = catalogs else {
            return;
        };
        assert_eq!(
            catalogs
                .iter()
                .map(|catalog| (catalog.provider.id.as_str(), catalog.icons.len()))
                .collect::<Vec<_>>(),
            [
                ("aws", 305),
                ("gcp", 45),
                ("azure", 639),
                ("simple-icons", 62)
            ]
        );

        let mut all_ids = BTreeSet::new();
        for catalog in &catalogs {
            assert_eq!(catalog.catalog_version, "1.0");
            assert!(catalog.icons.iter().all(|icon| {
                icon.id.starts_with(&format!("{}:", catalog.provider.id))
                    && all_ids.insert(icon.id.as_str())
            }));
        }
        assert_eq!(all_ids.len(), 1_051);
    }

    #[test]
    fn previous_ids_and_requested_tool_ids_remain_available() {
        let catalogs = provider_catalogs().unwrap_or_default();
        let ids = catalogs
            .iter()
            .flat_map(|catalog| catalog.icons.iter().map(|icon| icon.id.as_str()))
            .collect::<BTreeSet<_>>();
        for expected in [
            "aws:s3",
            "aws:sqs",
            "aws:lambda",
            "aws:ec2",
            "aws:rds",
            "aws:dynamodb",
            "aws:eks",
            "gcp:cloud-run",
            "gcp:cloud-storage",
            "gcp:compute-engine",
            "gcp:gke",
            "gcp:bigquery",
            "gcp:cloud-sql",
            "gcp:serverless",
            "azure:virtual-machines",
            "azure:storage-accounts",
            "azure:azure-sql-database",
            "azure:aks",
            "azure:app-service",
            "simple-icons:github",
            "simple-icons:notion",
            "simple-icons:linear",
            "simple-icons:atlassian",
            "simple-icons:jira",
            "simple-icons:confluence",
        ] {
            assert!(ids.contains(expected), "missing {expected}");
        }
    }
}
