use crate::{task_local::*, tl_info};
use axum::{body::Body, http::Request, middleware::Next, response::Response};
use axum_client_ip::InsecureClientIp;
use chrono::Utc;
use nanoid::nanoid;

use crate::error::HTTPResult;
use crate::task_local::{clone_value_from_task_local, STARTED_AT, TRACE_ID};

pub async fn entry<B>(req: Request<B>, next: Next<B>) -> Response {
    // 设置请求处理开始时间
    STARTED_AT
        .scope(Utc::now().timestamp_millis(), async {
            TRACE_ID
                .scope(nanoid!(6), async { next.run(req).await })
                .await
        })
        .await
}

pub async fn access_log(
    InsecureClientIp(ip): InsecureClientIp,
    req: Request<Body>,
    next: Next<Body>,
) -> HTTPResult<Response> {
    let start_at = STARTED_AT.with(clone_value_from_task_local);
    let uri = req.uri().to_string();
    let method = req.method().to_string();

    let resp = next.run(req).await;

    let status = resp.status().as_u16();

    let cost = Utc::now().timestamp_millis() - start_at;
    tl_info!(
        category = "access",
        ip = ip.to_string(),
        method,
        uri,
        status,
        cost,
    );

    Ok(resp)
}
