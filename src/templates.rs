//! Curated templates pinned to the public Stack specification corpus.

use std::ffi::OsStr;

pub(crate) const DEFAULT_ID: &str = "hello-stack";

#[derive(Clone, Copy)]
pub(crate) struct Template {
    pub(crate) id: &'static str,
    pub(crate) source: &'static [u8],
    pub(crate) providers: &'static [&'static str],
}

pub(crate) const ALL: &[Template] = &[
    Template {
        id: "hello-stack",
        source: include_bytes!("../templates/sources/01-minimal.stack"),
        providers: &[],
    },
    Template {
        id: "application-and-data",
        source: include_bytes!("../templates/sources/02-node-semantics.stack"),
        providers: &[],
    },
    Template {
        id: "groups-and-layout",
        source: include_bytes!("../templates/sources/03-groups-and-layout.stack"),
        providers: &[],
    },
    Template {
        id: "commerce-platform",
        source: include_bytes!("../templates/sources/04-commerce-platform.stack"),
        providers: &[],
    },
    Template {
        id: "aws-serverless-checkout",
        source: include_bytes!("../templates/sources/05-aws-serverless.stack"),
        providers: &["aws"],
    },
    Template {
        id: "gcp-data-service",
        source: include_bytes!("../templates/sources/06-gcp-data-service.stack"),
        providers: &["gcp"],
    },
    Template {
        id: "azure-event-platform",
        source: include_bytes!("../templates/sources/07-azure-event-platform.stack"),
        providers: &["azure"],
    },
    Template {
        id: "github-delivery-workflow",
        source: include_bytes!("../templates/sources/08-github-delivery.stack"),
        providers: &["simple-icons"],
    },
    Template {
        id: "mixed-provider-platform",
        source: include_bytes!("../templates/sources/09-mixed-provider-platform.stack"),
        providers: &["aws", "gcp", "azure", "simple-icons"],
    },
];

pub(crate) fn find(id: &OsStr) -> Option<Template> {
    ALL.iter()
        .copied()
        .find(|template| id == OsStr::new(template.id))
}
