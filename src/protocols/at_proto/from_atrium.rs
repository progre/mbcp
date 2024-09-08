use anyhow::{anyhow, Result};
use atrium_api::{
    app::{
        self,
        bsky::{
            embed::{record::ViewRecordRefs, record_with_media::ViewMediaRefs},
            feed::defs::{FeedViewPostReasonRefs, PostViewEmbedRefs},
        },
    },
    record::KnownRecord,
    types::{
        TryFromUnknown,
        Union::{self, Refs, Unknown},
    },
};
use chrono::DateTime;
use regex::Regex;

use crate::{sources::source, store};

impl TryFrom<app::bsky::richtext::facet::Main> for store::operations::Facet {
    type Error = anyhow::Error;

    fn try_from(value: app::bsky::richtext::facet::Main) -> Result<Self> {
        assert_eq!(value.features.len(), 1);
        let feature = &value.features[0];
        match feature {
            Refs(app::bsky::richtext::facet::MainFeaturesItem::Mention(mention)) => {
                Err(anyhow!("mention is not implemented: {:?}", mention))
            }
            Refs(app::bsky::richtext::facet::MainFeaturesItem::Link(link)) => {
                Ok(store::operations::Facet::Link {
                    byte_slice: (value.index.byte_start as u32)..(value.index.byte_end as u32),
                    uri: link.uri.clone(),
                })
            }
            Refs(app::bsky::richtext::facet::MainFeaturesItem::Tag(tag)) => {
                Err(anyhow!("tag is not implemented: {:?}", tag))
            }
            Unknown(_) => Err(anyhow!("unknown feature type")),
        }
    }
}

impl From<app::bsky::embed::images::ViewImage> for store::operations::Medium {
    fn from(value: app::bsky::embed::images::ViewImage) -> Self {
        store::operations::Medium {
            alt: value.alt.clone(),
            url: value.fullsize.clone(),
        }
    }
}

impl From<Box<app::bsky::embed::external::View>> for source::LiveExternal {
    fn from(value: Box<app::bsky::embed::external::View>) -> Self {
        source::LiveExternal::Some(store::operations::External {
            uri: value.external.uri.clone(),
            title: value.external.title.clone(),
            description: value.external.description.clone(),
            thumb_url: value.external.thumb.clone(),
        })
    }
}

fn to_external_uri(at_uri: &str) -> String {
    let m = Regex::new(r"^at://(.+?)/app.bsky.feed.post/(.+?)$")
        .unwrap()
        .captures(at_uri)
        .unwrap();
    format!(
        "https://bsky.app/profile/{}/post/{}",
        m.get(1).unwrap().as_str(),
        m.get(2).unwrap().as_str(),
    )
}

fn rewrite_content(
    mut content: String,
    mut facets: Option<Vec<app::bsky::richtext::facet::Main>>,
    quote: Option<&str>,
) -> String {
    if let Some(facets) = &mut facets {
        facets.sort_by_key(|x| x.index.byte_start);
        facets.reverse();
        for facet in facets {
            let Some(link) = facet
                .features
                .iter()
                .filter_map(|x| match x {
                    Refs(app::bsky::richtext::facet::MainFeaturesItem::Link(link)) => Some(link),
                    Refs(app::bsky::richtext::facet::MainFeaturesItem::Mention(_)) => None,
                    Refs(app::bsky::richtext::facet::MainFeaturesItem::Tag(_)) => None,
                    Unknown(_) => None,
                })
                .next()
            else {
                continue;
            };
            content.replace_range(facet.index.byte_start..facet.index.byte_end, &link.uri);
        }
    }
    if let Some(quote) = quote {
        if !content.contains(quote) {
            content.push_str("\n\n");
            content.push_str(quote);
        }
    }
    content
}

fn parse_embed(
    embed: Option<Union<PostViewEmbedRefs>>,
) -> (
    Vec<store::operations::Medium>,
    source::LiveExternal,
    Option<String>,
) {
    match embed {
        Some(Union::Refs(PostViewEmbedRefs::AppBskyEmbedImagesView(images))) => (
            images.data.images.into_iter().map(|x| x.into()).collect(),
            source::LiveExternal::None,
            None,
        ),
        Some(Union::Refs(PostViewEmbedRefs::AppBskyEmbedExternalView(external))) => {
            (vec![], external.into(), None)
        }
        Some(Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordView(embed))) => {
            match embed.data.record {
                Refs(ViewRecordRefs::ViewRecord(record)) => (
                    vec![],
                    source::LiveExternal::None,
                    Some(to_external_uri(&record.uri)),
                ),
                Refs(
                    ViewRecordRefs::ViewNotFound(_)
                    | ViewRecordRefs::ViewBlocked(_)
                    | ViewRecordRefs::AppBskyFeedDefsGeneratorView(_)
                    | ViewRecordRefs::AppBskyGraphDefsListView(_)
                    | ViewRecordRefs::AppBskyLabelerDefsLabelerView(_)
                    | ViewRecordRefs::AppBskyGraphDefsStarterPackViewBasic(_),
                )
                | Unknown(_) => (vec![], source::LiveExternal::None, None),
            }
        }
        Some(Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(embed))) => {
            let (media, external) = match embed.data.media {
                Refs(ViewMediaRefs::AppBskyEmbedImagesView(images)) => (
                    images.data.images.into_iter().map(|x| x.into()).collect(),
                    source::LiveExternal::None,
                ),
                Refs(ViewMediaRefs::AppBskyEmbedExternalView(external)) => {
                    (vec![], external.into())
                }
                Unknown(_) => (vec![], source::LiveExternal::None),
            };
            match embed.data.record.data.record {
                Refs(ViewRecordRefs::ViewRecord(record)) => {
                    (media, external, Some(to_external_uri(&record.uri)))
                }
                Refs(
                    ViewRecordRefs::ViewNotFound(_)
                    | ViewRecordRefs::ViewBlocked(_)
                    | ViewRecordRefs::AppBskyFeedDefsGeneratorView(_)
                    | ViewRecordRefs::AppBskyGraphDefsListView(_)
                    | ViewRecordRefs::AppBskyLabelerDefsLabelerView(_)
                    | ViewRecordRefs::AppBskyGraphDefsStarterPackViewBasic(_),
                )
                | Unknown(_) => (vec![], source::LiveExternal::None, None),
            }
        }
        Some(Union::Unknown(_)) | None => (vec![], source::LiveExternal::None, None),
    }
}

impl TryFrom<app::bsky::feed::defs::FeedViewPost> for source::LiveStatus {
    type Error = anyhow::Error;

    fn try_from(value: app::bsky::feed::defs::FeedViewPost) -> Result<Self> {
        let KnownRecord::AppBskyFeedPost(record) =
            KnownRecord::try_from_unknown(value.data.post.data.record)?
        else {
            unreachable!()
        };
        let (media, external, quote) = parse_embed(value.data.post.data.embed);
        Ok(
            if let Some(Union::Refs(FeedViewPostReasonRefs::ReasonRepost(reason))) =
                value.data.reason
            {
                source::LiveStatus::Repost(store::operations::CreateRepostOperationStatus {
                    src_identifier: value.data.post.data.cid.as_ref().to_string(),
                    target_src_identifier: value.data.post.data.cid.as_ref().to_string(),
                    target_src_uri: to_external_uri(&value.data.post.data.uri),
                    created_at: DateTime::parse_from_rfc3339(
                        &reason.indexed_at.as_ref().to_rfc3339(),
                    )?,
                })
            } else {
                let facets = record
                    .facets
                    .iter()
                    .flatten()
                    .filter_map(|x| x.to_owned().try_into().ok())
                    .collect();
                source::LiveStatus::Post(source::LivePost {
                    identifier: value.data.post.data.cid.as_ref().to_string(),
                    uri: value.data.post.data.uri.clone(),
                    content: rewrite_content(
                        record.text.to_owned(),
                        record.data.facets,
                        quote.as_deref(),
                    ),
                    facets,
                    reply_src_identifier: record
                        .data
                        .reply
                        .map(|x| x.parent.cid.as_ref().to_string()),
                    media,
                    external,
                    created_at: DateTime::parse_from_rfc3339(
                        &record.data.created_at.as_ref().to_rfc3339(),
                    )?,
                })
            },
        )
    }
}
