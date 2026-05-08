// Copyright 2026 Seungjin Kim
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use crate::wasi_http::http_request;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;
use wasi as bindings;

#[derive(Serialize)]
struct Value {
    #[serde(rename = "type")]
    value_type: String,
    value: String,
}

#[derive(Serialize)]
struct Stmt {
    sql: String,
    args: Vec<Value>,
}

#[derive(Serialize)]
struct Request {
    #[serde(rename = "type")]
    req_type: String,
    stmt: Stmt,
}

#[derive(Serialize)]
struct Pipeline {
    requests: Vec<Request>,
}

#[derive(Deserialize)]
struct PipelineResponse {
    results: Vec<serde_json::Value>,
}

async fn execute_sql(
    sql: String,
    args: Vec<Value>,
) -> Result<serde_json::Value> {
    let url_raw = env::var("TURSO_DATABASE_URL").expect("TURSO_DATABASE_URL not set");
    let mut url = url_raw.trim().to_string();
    if url.starts_with("libsql://") {
        url = url.replace("libsql://", "https://");
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("https://{}", url);
    }
    
    let token = env::var("TURSO_AUTH_TOKEN").expect("TURSO_AUTH_TOKEN not set");
    let token = token.trim();

    let pipeline = Pipeline {
        requests: vec![
            Request {
                req_type: "execute".to_string(),
                stmt: Stmt { sql: sql.clone(), args },
            },
            Request {
                req_type: "close".to_string(),
                stmt: Stmt {
                    sql: "".to_string(),
                    args: vec![],
                },
            },
        ],
    };

    let body = serde_json::to_vec(&pipeline)?;
    let headers = vec![
        (
            "Authorization".to_string(),
            format!("Bearer {}", token).into_bytes(),
        ),
        (
            "Content-Type".to_string(),
            "application/json".to_string().into_bytes(),
        ),
    ];

    let full_url = format!("{}/v2/pipeline", url.trim_end_matches('/'));
    let resp_body = http_request(
        bindings::http::types::Method::Post,
        &full_url,
        headers,
        Some(body),
    )
    .await?;

    let resp: PipelineResponse = serde_json::from_slice(&resp_body)?;
    let result = resp
        .results
        .get(0)
        .ok_or_else(|| anyhow::anyhow!("No results in pipeline response for SQL: {}", sql))?;

    if let Some(error) = result.get("error") {
        return Err(anyhow::anyhow!("Turso error for SQL '{}': {}", sql, error));
    }

    let response = result
        .get("response")
        .ok_or_else(|| anyhow::anyhow!("No response in pipeline result for SQL: {}", sql))?;
    
    Ok(response.clone())
}

pub async fn get_kv(key: &str) -> Result<Option<String>> {
    let table_name_raw = env::var("TURSO_KV_TABLE").unwrap_or_else(|_| "lachuoi_kv_store".to_string());
    let table_name = table_name_raw.trim();
    let table_name = if table_name.is_empty() { "lachuoi_kv_store" } else { table_name };

    // Ensure table exists
    let _ = execute_sql(
        format!("CREATE TABLE IF NOT EXISTS {} (key TEXT PRIMARY KEY, value TEXT, created_at DATETIME DEFAULT CURRENT_TIMESTAMP, updated_at DATETIME DEFAULT CURRENT_TIMESTAMP)", table_name),
        vec![],
    )
    .await?;

    let resp = execute_sql(
        format!("SELECT value FROM {} WHERE key = ?", table_name),
        vec![Value {
            value_type: "text".to_string(),
            value: key.to_string(),
        }],
    )
    .await?;

    // Try multiple pointers as Turso API versions vary
    // First try standard rows array structure
    if let Some(rows) = resp.pointer("/result/rows") {
        if let Some(first_row) = rows.get(0) {
            if let Some(first_col) = first_row.get(0) {
                // If it's an object with a "value" field (Turso Typed JSON)
                if let Some(val) = first_col.get("value") {
                    if let Some(s) = val.as_str() {
                        return Ok(Some(s.to_string()));
                    }
                    return Ok(Some(val.to_string()));
                }
                // If it's a direct value
                if let Some(s) = first_col.as_str() {
                    return Ok(Some(s.to_string()));
                }
                return Ok(Some(first_col.to_string()));
            }
        }
    }
    
    Ok(None)
}

pub async fn set_kv(key: &str, value: &str) -> Result<()> {
    let table_name_raw = env::var("TURSO_KV_TABLE").unwrap_or_else(|_| "lachuoi_kv_store".to_string());
    let table_name = table_name_raw.trim();
    let table_name = if table_name.is_empty() { "lachuoi_kv_store" } else { table_name };

    println!("Setting KV in Turso: {} = {}", key, value);

    // Using separate calls for clarity in debugging, but checking all results
    let _ = execute_sql(
        format!("CREATE TABLE IF NOT EXISTS {} (key TEXT PRIMARY KEY, value TEXT, created_at DATETIME DEFAULT CURRENT_TIMESTAMP, updated_at DATETIME DEFAULT CURRENT_TIMESTAMP)", table_name),
        vec![],
    )
    .await?;

    let _ = execute_sql(
        format!("INSERT INTO {} (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP", table_name),
        vec![
            Value {
                value_type: "text".to_string(),
                value: key.to_string(),
            },
            Value {
                value_type: "text".to_string(),
                value: value.to_string(),
            },
        ],
    )
    .await?;
    
    println!("KV set successfully: {}", key);
    Ok(())
}
