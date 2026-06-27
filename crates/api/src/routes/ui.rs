//! Minimal server-rendered operator UI (scope §18 path 1: Axum server-render + small JS
//! islands). Read views are plain HTML built from the DB (no template-engine dependency);
//! mutations go through the JSON API. The camera/scan + long-receipt capture are JS islands
//! that need a real device and a secure context (HTTPS/localhost) to run (scope §13.1).

use axum::extract::{Path, State};
use axum::response::Html;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::AppState;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map(esc).unwrap_or_default()
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>CEC Inventory — {title}</title>\
<style>body{{font-family:system-ui,sans-serif;margin:0;background:#0f1115;color:#e6e6e6}}\
header{{background:#161a22;padding:12px 20px;border-bottom:1px solid #2a2f3a}}\
header a{{color:#9ad;margin-right:16px;text-decoration:none}}main{{padding:20px;max-width:1100px}}\
h1{{font-size:20px}}table{{border-collapse:collapse;width:100%}}\
th,td{{text-align:left;padding:6px 10px;border-bottom:1px solid #2a2f3a;font-size:14px}}\
th{{color:#9aa}}.cards{{display:flex;gap:12px;flex-wrap:wrap}}\
.card{{background:#161a22;border:1px solid #2a2f3a;border-radius:8px;padding:16px 20px;min-width:120px}}\
.card .n{{font-size:28px;font-weight:600}}.card .l{{color:#9aa;font-size:13px}}\
code{{background:#222834;padding:1px 5px;border-radius:4px}}</style></head><body>\
<header><a href=\"/\">Dashboard</a><a href=\"/ui/units\">Units</a>\
<a href=\"/ui/systems\">Systems</a><a href=\"/ui/purchases\">Purchases</a>\
<a href=\"/ui/scan\">Scan</a></header><main>{body}</main></body></html>"
    ))
}

async fn count(s: &AppState, sql: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0)
}

pub async fn dashboard(State(s): State<AppState>) -> ApiResult<Html<String>> {
    let units = count(&s, "SELECT count(*) FROM inventory_unit").await;
    let systems = count(&s, "SELECT count(*) FROM system").await;
    let purchases = count(&s, "SELECT count(*) FROM purchase").await;
    let rmas = count(&s, "SELECT count(*) FROM rma_case WHERE status <> 'closed'").await;
    let reorder = count(
        &s,
        "SELECT count(*) FROM stock_item WHERE reorder_point IS NOT NULL AND quantity_on_hand <= reorder_point",
    )
    .await;
    let card = |n: i64, l: &str| {
        format!("<div class=\"card\"><div class=\"n\">{n}</div><div class=\"l\">{l}</div></div>")
    };
    let body = format!(
        "<h1>CEC Inventory</h1><div class=\"cards\">{}{}{}{}{}</div>\
<p style=\"margin-top:20px;color:#9aa\">Server-rendered operator view (scope §18). Mutations \
use the JSON API; the <a href=\"/ui/scan\">scan</a> page is a camera island needing a device \
+ HTTPS.</p>",
        card(units, "Units"),
        card(systems, "Systems"),
        card(purchases, "Purchases"),
        card(rmas, "Open RMAs"),
        card(reorder, "Reorder")
    );
    Ok(page("Dashboard", &body))
}

#[derive(FromRow)]
struct UnitRow {
    id: Uuid,
    serial_number: Option<String>,
    status: String,
    owner: String,
    model: Option<String>,
}

pub async fn units_page(State(s): State<AppState>) -> ApiResult<Html<String>> {
    let rows = sqlx::query_as::<_, UnitRow>(
        "SELECT u.id, u.serial_number, u.status::text AS status, u.owner::text AS owner, p.model \
         FROM inventory_unit u LEFT JOIN product p ON p.id = u.product_id \
         ORDER BY u.intake_at DESC LIMIT 200",
    )
    .fetch_all(&s.db)
    .await?;
    let mut t = String::from("<h1>Units</h1><table><tr><th>Serial</th><th>Product</th><th>Status</th><th>Owner</th><th>ID</th></tr>");
    for r in &rows {
        t.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
            opt(&r.serial_number),
            opt(&r.model),
            esc(&r.status),
            esc(&r.owner),
            r.id
        ));
    }
    t.push_str("</table>");
    Ok(page("Units", &t))
}

#[derive(FromRow)]
struct SystemRow {
    id: Uuid,
    label: Option<String>,
    status: String,
    validation_state: String,
    current_owner: String,
}

pub async fn systems_page(State(s): State<AppState>) -> ApiResult<Html<String>> {
    let rows = sqlx::query_as::<_, SystemRow>(
        "SELECT id, label, status::text AS status, validation_state::text AS validation_state, \
         current_owner::text AS current_owner FROM system ORDER BY id DESC LIMIT 200",
    )
    .fetch_all(&s.db)
    .await?;
    let mut t = String::from("<h1>Systems</h1><table><tr><th>Label</th><th>Status</th><th>Validation</th><th>Owner</th><th>ID</th></tr>");
    for r in &rows {
        t.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
            opt(&r.label),
            esc(&r.status),
            esc(&r.validation_state),
            esc(&r.current_owner),
            r.id
        ));
    }
    t.push_str("</table>");
    Ok(page("Systems", &t))
}

#[derive(FromRow)]
struct PurchaseRow {
    id: Uuid,
    source_type: String,
    total: Option<rust_decimal::Decimal>,
    created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn purchases_page(State(s): State<AppState>) -> ApiResult<Html<String>> {
    let rows = sqlx::query_as::<_, PurchaseRow>(
        "SELECT id, source_type::text AS source_type, total, created_at FROM purchase \
         ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&s.db)
    .await?;
    let mut t = String::from("<h1>Purchases</h1><table><tr><th>Source</th><th>Total</th><th>Created</th><th>ID</th></tr>");
    for r in &rows {
        t.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
            esc(&r.source_type),
            r.total.map(|v| v.to_string()).unwrap_or_default(),
            r.created_at.format("%Y-%m-%d %H:%M"),
            r.id
        ));
    }
    t.push_str("</table>");
    Ok(page("Purchases", &t))
}

/// Camera + barcode scan island (scope §13.1). Uses native BarcodeDetector where present;
/// a WASM fallback (zxing-wasm) is wired for Safari/iOS in a later pass. Needs a secure
/// context. Posts the scanned serial to `POST /units/{id}/verify`.
pub async fn scan_page(Path(unit_id): Path<Uuid>) -> Html<String> {
    let body = format!(
        "<h1>Scan to verify</h1><p class=\"l\">Unit <code>{unit_id}</code></p>\
<video id=\"v\" autoplay playsinline muted style=\"max-width:420px;width:100%;border-radius:8px\"></video>\
<p id=\"out\" style=\"color:#9aa\">Point the rear camera at the serial barcode…</p>\
<script>\
const unitId={js_id:?};\
async function run(){{\
  const out=document.getElementById('out');\
  if(!('BarcodeDetector' in window)){{out.textContent='Native BarcodeDetector unavailable; a WASM fallback (zxing-wasm) ships later for Safari/iOS.';return;}}\
  const det=new BarcodeDetector({{formats:['code_128','code_39','qr_code','data_matrix','ean_13','upc_a']}});\
  let stream;try{{stream=await navigator.mediaDevices.getUserMedia({{video:{{facingMode:'environment'}}}});}}catch(e){{out.textContent='Camera needs HTTPS/localhost and permission: '+e;return;}}\
  const v=document.getElementById('v');v.srcObject=stream;\
  const tick=async()=>{{try{{const codes=await det.detect(v);if(codes.length){{const serial=codes[0].rawValue;out.textContent='Scanned '+serial+' — verifying…';\
    const r=await fetch('/units/'+unitId+'/verify',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify({{scanned_serial:serial}})}});\
    const j=await r.json();out.textContent='verified='+j.verified+' matched='+j.matched+(j.warnings&&j.warnings.length?' ⚠ '+j.warnings.join('; '):'');stream.getTracks().forEach(t=>t.stop());return;}}}}catch(e){{}}requestAnimationFrame(tick);}};\
  requestAnimationFrame(tick);\
}}run();\
</script>",
        js_id = unit_id.to_string()
    );
    page("Scan", &body)
}

/// Landing scan page without a target unit, explaining the island.
pub async fn scan_index() -> Html<String> {
    page(
        "Scan",
        "<h1>Scan</h1><p class=\"l\">Open <code>/ui/scan/&lt;unit-id&gt;</code> on a phone (HTTPS \
or localhost) to verify a unit's serial with the camera (scope §13.1). Long-receipt guided \
capture and the WASM fallback for Safari/iOS land in a later pass.</p>",
    )
}
