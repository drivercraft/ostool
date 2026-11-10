//! Explicitly public device documentation stored in PostgreSQL.
use super::Database;
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceDocument {
    pub title: String,
    pub summary: String,
    pub description: String,
    pub image_url: String,
    pub highlights: Vec<String>,
    pub specifications: Vec<Specification>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Specification {
    pub name: String,
    pub value: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct DocumentView {
    pub id: String,
    pub revision: i64,
    pub document: DeviceDocument,
}
#[derive(Serialize)]
pub struct DocumentSummary {
    pub title: String,
    pub summary: String,
    pub image_url: String,
    pub revision: i64,
}
impl DeviceDocument {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.title.len() <= 160
                && self.summary.len() <= 600
                && self.description.len() <= 16000
                && self.highlights.len() <= 8
                && self.specifications.len() <= 32,
            "公开文档内容超出长度限制"
        );
        ensure!(
            self.highlights
                .iter()
                .all(|v| !v.trim().is_empty() && v.len() <= 240),
            "亮点不能为空且每项最多 240 字节"
        );
        ensure!(
            self.specifications.iter().all(|v| !v.name.trim().is_empty()
                && !v.value.trim().is_empty()
                && v.name.len() <= 80
                && v.value.len() <= 400),
            "参数名称和值不能为空或超出长度限制"
        );
        ensure!(self.image_url.len() <= 2048, "图片地址过长");
        if !self.image_url.is_empty() {
            let relative = self.image_url.starts_with('/')
                && !self.image_url.starts_with("//")
                && !self.image_url.contains(['\\', '\n', '\r']);
            let https = url::Url::parse(&self.image_url).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            });
            ensure!(relative || https, "图片必须使用站内路径或 HTTPS 地址");
        }
        Ok(())
    }
}
impl Database {
    pub async fn device_document(&self, id: &str) -> anyhow::Result<DocumentView> {
        let row = sqlx::query("SELECT revision,document FROM device_documents WHERE board_id=$1")
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(DocumentView {
                id: id.into(),
                revision: row.try_get("revision")?,
                document: serde_json::from_value(row.try_get("document")?)?,
            }),
            None => Ok(DocumentView {
                id: id.into(),
                revision: 0,
                document: DeviceDocument::default(),
            }),
        }
    }
    pub async fn device_document_summaries(
        &self,
    ) -> anyhow::Result<BTreeMap<String, DocumentSummary>> {
        let rows=sqlx::query("SELECT board_id,revision,document->>'title' title,document->>'summary' summary,document->>'image_url' image_url FROM device_documents ORDER BY board_id").fetch_all(self.pool()).await?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.try_get("board_id")?,
                    DocumentSummary {
                        title: row.try_get("title")?,
                        summary: row.try_get("summary")?,
                        image_url: row.try_get("image_url")?,
                        revision: row.try_get("revision")?,
                    },
                ))
            })
            .collect()
    }
    /// A stale editor cannot overwrite a later publication.
    pub async fn save_device_document(
        &self,
        id: &str,
        revision: i64,
        document: &DeviceDocument,
    ) -> anyhow::Result<bool> {
        document.validate()?;
        if revision > 0 {
            return Ok(sqlx::query("UPDATE device_documents SET revision=revision+1,document=$3,updated_at=now() WHERE board_id=$1 AND revision=$2").bind(id).bind(revision).bind(serde_json::to_value(document)?).execute(self.pool()).await?.rows_affected()==1);
        }
        let result=sqlx::query("INSERT INTO device_documents(board_id,revision,document) VALUES($1,1,$2) ON CONFLICT(board_id) DO NOTHING")
            .bind(id).bind(serde_json::to_value(document)?).execute(self.pool()).await?;
        Ok(result.rows_affected() == 1)
    }
}
