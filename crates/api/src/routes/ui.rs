//! Server-rendered operator UI (scope §18 path 1: Axum server-render + small JS islands).
//!
//! Read views are plain HTML built from the DB (no template-engine dependency). Mutations go
//! through the JSON API: every form serializes its fields to JSON and POSTs with the browser's
//! signed session cookie (so the `require_auth` data routes accept it once logged in). A single
//! shared `cecSubmit` helper (in the page `<head>`) does the serialize-and-POST and renders the
//! result inline. The camera/scan island (scope §13.1) needs a real device + secure context.

use axum::extract::{Path, State};
use axum::response::Html;
use axum_extra::extract::cookie::SignedCookieJar;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::SESSION_COOKIE;
use crate::error::ApiResult;
use crate::AppState;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map(esc).unwrap_or_default()
}

/// Shared client helper. Serializes a form's `[name]` fields to JSON, honoring
/// `data-type` (`number`/`bool`/`lines` → string array), omitting empty optionals, and POSTs
/// (or PATCHes) to `form.action` with the session cookie. Renders the response inline and,
/// on success, optionally redirects/reloads (`data-redirect` / `data-reload`).
const SCRIPT: &str = r#"
async function cecSubmit(form){
  const out = form.querySelector('.result') || document.getElementById('result');
  const data = {};
  form.querySelectorAll('[name]').forEach(el=>{
    if(el.disabled) return;
    const t = el.dataset.type, name = el.name;
    if(el.type==='checkbox'){ data[name]=el.checked; return; }
    let v = el.value;
    if(t==='lines'){ const arr=v.split(/[\n,]+/).map(x=>x.trim()).filter(Boolean); if(arr.length||el.dataset.required) data[name]=arr; return; }
    if(v===''){ if(!el.dataset.required) return; }
    if(t==='number'){ if(v==='') return; v=Number(v); }
    data[name]=v;
  });
  const method = form.dataset.method || 'POST';
  if(out){ out.textContent='…'; out.className='result'; }
  try{
    const r = await fetch(form.action,{method,headers:{'content-type':'application/json'},body:JSON.stringify(data)});
    const text = await r.text();
    let pretty=text; try{ pretty=JSON.stringify(JSON.parse(text),null,2); }catch(e){}
    if(out){ out.textContent=(r.ok?'✓ ':'✗ ')+r.status+'\n'+pretty; out.className='result '+(r.ok?'ok':'err'); }
    if(r.ok && form.dataset.redirect){ location.href=form.dataset.redirect; }
    else if(r.ok && form.dataset.reload){ setTimeout(()=>location.reload(),700); }
  }catch(e){ if(out){ out.textContent='✗ '+e; out.className='result err'; } }
  return false;
}
async function cecLogout(){ await fetch('/auth/logout',{method:'POST'}); location.href='/'; }
"#;

fn nav(user: Option<&str>) -> String {
    let right = match user {
        Some(u) => format!(
            "<span style=\"float:right;color:#9aa\">{} · \
<a href=\"#\" onclick=\"cecLogout();return false\">Logout</a></span>",
            esc(u)
        ),
        None => "<span style=\"float:right\"><a href=\"/ui/login\">Login</a></span>".to_string(),
    };
    format!(
        "<header><a href=\"/\">Dashboard</a><a href=\"/ui/units\">Units</a>\
<a href=\"/ui/systems\">Systems</a><a href=\"/ui/purchases\">Purchases</a>\
<a href=\"/ui/new\">New entry</a><a href=\"/ui/scan\">Scan</a>{right}</header>"
    )
}

fn page(title: &str, user: Option<&str>, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>CEC Inventory — {title}</title>\
<style>body{{font-family:system-ui,sans-serif;margin:0;background:#0f1115;color:#e6e6e6}}\
header{{background:#161a22;padding:12px 20px;border-bottom:1px solid #2a2f3a}}\
header a{{color:#9ad;margin-right:16px;text-decoration:none}}main{{padding:20px;max-width:1100px}}\
h1{{font-size:20px}}h2{{font-size:16px;margin-top:28px;border-bottom:1px solid #2a2f3a;padding-bottom:6px}}\
table{{border-collapse:collapse;width:100%}}\
th,td{{text-align:left;padding:6px 10px;border-bottom:1px solid #2a2f3a;font-size:14px}}\
th{{color:#9aa}}.cards{{display:flex;gap:12px;flex-wrap:wrap}}\
.card{{background:#161a22;border:1px solid #2a2f3a;border-radius:8px;padding:16px 20px;min-width:120px}}\
.card .n{{font-size:28px;font-weight:600}}.card .l{{color:#9aa;font-size:13px}}\
code{{background:#222834;padding:1px 5px;border-radius:4px}}a{{color:#9ad}}\
form.cec{{background:#161a22;border:1px solid #2a2f3a;border-radius:8px;padding:14px 16px;margin:14px 0;max-width:560px}}\
form.cec label{{display:block;color:#9aa;font-size:13px;margin:8px 0 2px}}\
input,select,textarea{{width:100%;box-sizing:border-box;background:#0f1115;color:#e6e6e6;\
border:1px solid #2a2f3a;border-radius:5px;padding:7px 9px;font-size:14px}}\
input[type=checkbox]{{width:auto}}\
button{{margin-top:12px;background:#274; color:#dfe; border:1px solid #386;border-radius:6px;\
padding:8px 14px;font-size:14px;cursor:pointer}}button:hover{{background:#386}}\
.result{{white-space:pre-wrap;font-family:ui-monospace,monospace;font-size:12px;margin-top:10px;\
color:#9aa;max-height:240px;overflow:auto}}.result.ok{{color:#8d8}}.result.err{{color:#e99}}\
.row{{display:flex;gap:8px}}.row>*{{flex:1}}.muted{{color:#9aa}}</style>\
<script>{script}</script></head><body>{nav}<main>{body}</main></body></html>",
        script = SCRIPT,
        nav = nav(user),
    ))
}

/// Resolve the logged-in operator (or None) from the signed session cookie.
async fn current_user(s: &AppState, jar: &SignedCookieJar) -> Option<String> {
    let uid = jar
        .get(SESSION_COOKIE)
        .and_then(|c| Uuid::parse_str(c.value()).ok())?;
    sqlx::query_scalar::<_, String>("SELECT username FROM app_user WHERE id = $1")
        .bind(uid)
        .fetch_optional(&s.db)
        .await
        .ok()
        .flatten()
}

async fn count(s: &AppState, sql: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0)
}

/// `(id, label)` options for a `<select>`, built from a query returning a uuid + text.
async fn options(s: &AppState, sql: &str, placeholder: &str) -> String {
    let rows = sqlx::query_as::<_, (Uuid, Option<String>)>(sql)
        .fetch_all(&s.db)
        .await
        .unwrap_or_default();
    let mut o = format!("<option value=\"\">{}</option>", esc(placeholder));
    for (id, label) in rows {
        let l = label.unwrap_or_else(|| id.to_string());
        o.push_str(&format!("<option value=\"{id}\">{}</option>", esc(&l)));
    }
    o
}

/// `<option>`s from a static enum variant list (snake_case PG values).
fn enum_options(variants: &[&str]) -> String {
    variants
        .iter()
        .map(|v| format!("<option value=\"{v}\">{v}</option>"))
        .collect()
}

// ---------------------------------------------------------------------------
// read views
// ---------------------------------------------------------------------------

pub async fn dashboard(State(s): State<AppState>, jar: SignedCookieJar) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
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
    let hint = if user.is_some() {
        "Logged in — manual entry and workflow actions are live."
    } else {
        "Read-only until you <a href=\"/ui/login\">log in</a>; mutations need a session."
    };
    let body = format!(
        "<h1>CEC Inventory</h1><div class=\"cards\">{}{}{}{}{}</div>\
<p style=\"margin-top:20px\" class=\"muted\">Server-rendered operator view (scope §18). {hint}<br>\
Start with <a href=\"/ui/new\">New entry</a> (catalog → unit/stock) or a \
<a href=\"/ui/purchases/new\">new purchase</a>.</p>",
        card(units, "Units"),
        card(systems, "Systems"),
        card(purchases, "Purchases"),
        card(rmas, "Open RMAs"),
        card(reorder, "Reorder")
    );
    Ok(page("Dashboard", user.as_deref(), &body))
}

#[derive(FromRow)]
struct UnitRow {
    id: Uuid,
    serial_number: Option<String>,
    status: String,
    owner: String,
    model: Option<String>,
}

pub async fn units_page(
    State(s): State<AppState>,
    jar: SignedCookieJar,
) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    let rows = sqlx::query_as::<_, UnitRow>(
        "SELECT u.id, u.serial_number, u.status::text AS status, u.owner::text AS owner, p.model \
         FROM inventory_unit u LEFT JOIN product p ON p.id = u.product_id \
         ORDER BY u.intake_at DESC LIMIT 200",
    )
    .fetch_all(&s.db)
    .await?;
    let mut t = String::from(
        "<h1>Units</h1><p class=\"muted\"><a href=\"/ui/new\">+ New unit</a></p>\
<table><tr><th>Serial</th><th>Product</th><th>Status</th><th>Owner</th><th></th></tr>",
    );
    for r in &rows {
        t.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
<td><a href=\"/ui/units/{}\">open</a></td></tr>",
            opt(&r.serial_number),
            opt(&r.model),
            esc(&r.status),
            esc(&r.owner),
            r.id
        ));
    }
    t.push_str("</table>");
    Ok(page("Units", user.as_deref(), &t))
}

#[derive(FromRow)]
struct SystemRow {
    id: Uuid,
    label: Option<String>,
    status: String,
    validation_state: String,
    current_owner: String,
}

pub async fn systems_page(
    State(s): State<AppState>,
    jar: SignedCookieJar,
) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    let rows = sqlx::query_as::<_, SystemRow>(
        "SELECT id, label, status::text AS status, validation_state::text AS validation_state, \
         current_owner::text AS current_owner FROM system ORDER BY id DESC LIMIT 200",
    )
    .fetch_all(&s.db)
    .await?;
    let mut t = String::from(
        "<h1>Systems</h1><p class=\"muted\"><a href=\"/ui/new\">+ New system</a></p>\
<table><tr><th>Label</th><th>Status</th><th>Validation</th><th>Owner</th><th></th></tr>",
    );
    for r in &rows {
        t.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
<td><a href=\"/ui/systems/{}\">open</a></td></tr>",
            opt(&r.label),
            esc(&r.status),
            esc(&r.validation_state),
            esc(&r.current_owner),
            r.id
        ));
    }
    t.push_str("</table>");
    Ok(page("Systems", user.as_deref(), &t))
}

#[derive(FromRow)]
struct PurchaseRow {
    id: Uuid,
    source_type: String,
    total: Option<rust_decimal::Decimal>,
    created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn purchases_page(
    State(s): State<AppState>,
    jar: SignedCookieJar,
) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    let rows = sqlx::query_as::<_, PurchaseRow>(
        "SELECT id, source_type::text AS source_type, total, created_at FROM purchase \
         ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&s.db)
    .await?;
    let mut t = String::from(
        "<h1>Purchases</h1><p class=\"muted\"><a href=\"/ui/purchases/new\">+ New purchase</a></p>\
<table><tr><th>Source</th><th>Total</th><th>Created</th><th>ID</th></tr>",
    );
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
    Ok(page("Purchases", user.as_deref(), &t))
}

// ---------------------------------------------------------------------------
// login / bootstrap
// ---------------------------------------------------------------------------

pub async fn login_page(
    State(s): State<AppState>,
    jar: SignedCookieJar,
) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    if let Some(u) = &user {
        let body = format!(
            "<h1>Login</h1><p>Logged in as <b>{}</b>. \
<a href=\"#\" onclick=\"cecLogout();return false\">Log out</a>.</p>",
            esc(u)
        );
        return Ok(page("Login", user.as_deref(), &body));
    }
    let users: i64 = count(&s, "SELECT count(*) FROM app_user").await;
    let body = if users == 0 {
        // No operators yet: offer the one-time bootstrap of the first account.
        "<h1>First-run setup</h1><p class=\"muted\">No operators exist yet. Create the first \
account (allowed once, then use authenticated user creation).</p>\
<form class=\"cec\" action=\"/auth/bootstrap\" data-redirect=\"/\" onsubmit=\"return cecSubmit(this)\">\
<label>Username</label><input name=\"username\" data-required=\"1\" autocomplete=\"username\">\
<label>Password (≥8 chars)</label><input name=\"password\" type=\"password\" data-required=\"1\" autocomplete=\"new-password\">\
<button>Create first operator</button><pre class=\"result\"></pre></form>"
            .to_string()
    } else {
        "<h1>Login</h1>\
<form class=\"cec\" action=\"/auth/login\" data-redirect=\"/\" onsubmit=\"return cecSubmit(this)\">\
<label>Username</label><input name=\"username\" data-required=\"1\" autocomplete=\"username\">\
<label>Password</label><input name=\"password\" type=\"password\" data-required=\"1\" autocomplete=\"current-password\">\
<button>Log in</button><pre class=\"result\"></pre></form>"
            .to_string()
    };
    Ok(page("Login", None, &body))
}

// ---------------------------------------------------------------------------
// manual entry hub
// ---------------------------------------------------------------------------

pub async fn new_entry(State(s): State<AppState>, jar: SignedCookieJar) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    let mfr_opts = options(
        &s,
        "SELECT id, name FROM manufacturer ORDER BY name",
        "— none —",
    )
    .await;
    let prod_opts = options(
        &s,
        "SELECT id, model FROM product ORDER BY model",
        "— product —",
    )
    .await;

    let gate = if user.is_none() {
        "<p class=\"result err\">Not logged in — these forms will 401 until you \
<a href=\"/ui/login\">log in</a>.</p>"
    } else {
        ""
    };

    let condition = enum_options(&["new", "open_box", "used", "refurb", "unknown"]);
    let serial_src = enum_options(&["receipt", "scan", "ocr", "manual"]);
    let status = enum_options(&[
        "in_stock",
        "reserved",
        "in_build",
        "installed",
        "with_customer",
        "shipped",
    ]);

    let body = format!(
        "<h1>New entry</h1>{gate}\
<p class=\"muted\">Each form POSTs JSON to the API with your session cookie. \
Build the catalog first (manufacturer → product), then add units/stock.</p>\
\
<h2>Vendor</h2>\
<form class=\"cec\" action=\"/vendors\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Name</label><input name=\"name\" data-required=\"1\">\
<label>Website</label><input name=\"website\">\
<label>RMA URL</label><input name=\"rma_url\">\
<label>Notes</label><input name=\"notes\">\
<button>Create vendor</button><pre class=\"result\"></pre></form>\
\
<h2>Manufacturer</h2>\
<form class=\"cec\" action=\"/manufacturers\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Name</label><input name=\"name\" data-required=\"1\">\
<label>Default warranty (months)</label><input name=\"default_warranty_months\" type=\"number\" data-type=\"number\">\
<label>Replacement warranty (days)</label><input name=\"replacement_warranty_days\" type=\"number\" data-type=\"number\">\
<label>Warranty policy URL</label><input name=\"warranty_policy_url\">\
<button>Create manufacturer</button><pre class=\"result\"></pre></form>\
\
<h2>Product</h2>\
<form class=\"cec\" action=\"/products\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Model</label><input name=\"model\" data-required=\"1\">\
<label>Manufacturer</label><select name=\"manufacturer_id\">{mfr_opts}</select>\
<label>MPN</label><input name=\"mpn\">\
<label>UPC/EAN</label><input name=\"upc_ean\">\
<label>Category</label><input name=\"category\">\
<label>Default warranty (months)</label><input name=\"default_warranty_months\" type=\"number\" data-type=\"number\">\
<label>Serial format regex (optional)</label><input name=\"serial_format_regex\">\
<label><input type=\"checkbox\" name=\"serialized\" checked> Serialized</label>\
<button>Create product</button><pre class=\"result\"></pre></form>\
\
<h2>Serialized unit</h2>\
<form class=\"cec\" action=\"/units\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Product</label><select name=\"product_id\" data-required=\"1\">{prod_opts}</select>\
<label>Serial number</label><input name=\"serial_number\">\
<div class=\"row\"><div><label>Serial source</label><select name=\"serial_source\"><option value=\"\">—</option>{serial_src}</select></div>\
<div><label>Condition</label><select name=\"condition\">{condition}</select></div></div>\
<div class=\"row\"><div><label>Status</label><select name=\"status\">{status}</select></div>\
<div><label>Unit cost</label><input name=\"unit_cost\" placeholder=\"e.g. 129.00\"></div></div>\
<label>Location bin</label><input name=\"location_bin\">\
<label>Notes</label><input name=\"notes\">\
<button>Create unit</button><pre class=\"result\"></pre></form>\
\
<h2>Bulk stock</h2>\
<form class=\"cec\" action=\"/stock\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Product</label><select name=\"product_id\" data-required=\"1\">{prod_opts}</select>\
<div class=\"row\"><div><label>Quantity on hand</label><input name=\"quantity_on_hand\" type=\"number\" data-type=\"number\" value=\"0\"></div>\
<div><label>Reorder point</label><input name=\"reorder_point\" type=\"number\" data-type=\"number\"></div></div>\
<label>Location bin</label><input name=\"location_bin\">\
<button>Create stock item</button><pre class=\"result\"></pre></form>\
\
<h2>System (build)</h2>\
<form class=\"cec\" action=\"/systems\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Label</label><input name=\"label\">\
<label>Notes</label><input name=\"notes\">\
<button>Create system</button><pre class=\"result\"></pre></form>",
    );
    Ok(page("New entry", user.as_deref(), &body))
}

// ---------------------------------------------------------------------------
// new purchase (header + repeatable line items)
// ---------------------------------------------------------------------------

pub async fn new_purchase(
    State(s): State<AppState>,
    jar: SignedCookieJar,
) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    let vendor_opts = options(
        &s,
        "SELECT id, name FROM vendor ORDER BY name",
        "— vendor —",
    )
    .await;
    let prod_opts = options(
        &s,
        "SELECT id, model FROM product ORDER BY model",
        "— map later —",
    )
    .await;
    let source = enum_options(&[
        "manual",
        "physical_photo",
        "pdf",
        "email",
        "trade_in",
        "opening_balance",
    ]);
    let gate = if user.is_none() {
        "<p class=\"result err\">Not logged in — this will 401 until you \
<a href=\"/ui/login\">log in</a>.</p>"
    } else {
        ""
    };
    // Dedicated script: gathers the header + each .li row into the nested CreatePurchase JSON.
    let body = format!(
        "<h1>New purchase</h1>{gate}\
<form class=\"cec\" id=\"pf\" onsubmit=\"return submitPurchase(this)\" style=\"max-width:760px\">\
<div class=\"row\"><div><label>Vendor</label><select name=\"vendor_id\">{vendor_opts}</select></div>\
<div><label>Source</label><select name=\"source_type\">{source}</select></div></div>\
<div class=\"row\"><div><label>Order #</label><input name=\"order_number\"></div>\
<div><label>Invoice #</label><input name=\"invoice_number\"></div></div>\
<div class=\"row\"><div><label>Subtotal</label><input name=\"subtotal\"></div>\
<div><label>Tax</label><input name=\"tax\"></div><div><label>Shipping</label><input name=\"shipping\"></div>\
<div><label>Total</label><input name=\"total\"></div></div>\
<h2>Line items</h2><div id=\"lines\"></div>\
<button type=\"button\" onclick=\"addLine()\">+ Add line</button> \
<button type=\"submit\">Create purchase</button><pre class=\"result\"></pre></form>\
<script>\
function addLine(){{\
  const d=document.createElement('div');d.className='li';d.style='border-top:1px solid #2a2f3a;padding-top:8px;margin-top:8px';\
  d.innerHTML='<label>Description (as printed)</label><input class=\"desc\">'+\
    '<div class=\"row\"><div><label>Product (optional)</label><select class=\"prod\">{prod_opts}</select></div>'+\
    '<div><label>Qty</label><input class=\"qty\" type=\"number\" value=\"1\"></div>'+\
    '<div><label>Unit price</label><input class=\"up\"></div>'+\
    '<div><label>Line total</label><input class=\"lt\"></div></div>'+\
    '<button type=\"button\" onclick=\"this.parentNode.remove()\">remove line</button>';\
  document.getElementById('lines').appendChild(d);\
}}\
addLine();\
async function submitPurchase(form){{\
  const g=n=>{{const e=form.querySelector('[name='+n+']');return e&&e.value!==''?e.value:undefined;}};\
  const body={{}};\
  ['vendor_id','source_type','order_number','invoice_number','subtotal','tax','shipping','total'].forEach(n=>{{const v=g(n);if(v!==undefined)body[n]=v;}});\
  body.line_items=[...document.querySelectorAll('#lines .li')].map(li=>{{\
    const o={{quantity:Number(li.querySelector('.qty').value||'1')}};\
    const desc=li.querySelector('.desc').value;if(desc)o.description_as_printed=desc;\
    const pid=li.querySelector('.prod').value;if(pid)o.product_id=pid;\
    const up=li.querySelector('.up').value;if(up)o.unit_price=up;\
    const lt=li.querySelector('.lt').value;if(lt)o.line_total=lt;\
    return o;}});\
  const out=form.querySelector('.result');out.textContent='…';out.className='result';\
  const r=await fetch('/purchases',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify(body)}});\
  const text=await r.text();let pretty=text;try{{pretty=JSON.stringify(JSON.parse(text),null,2);}}catch(e){{}}\
  out.textContent=(r.ok?'✓ ':'✗ ')+r.status+'\\n'+pretty;out.className='result '+(r.ok?'ok':'err');\
  if(r.ok)setTimeout(()=>location.href='/ui/purchases',900);\
  return false;\
}}\
</script>"
    );
    Ok(page("New purchase", user.as_deref(), &body))
}

// ---------------------------------------------------------------------------
// unit detail + actions
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct UnitDetail {
    serial_number: Option<String>,
    status: String,
    owner: String,
    condition: String,
    verified: bool,
    asset_tag: Option<String>,
    model: Option<String>,
    system_id: Option<Uuid>,
}

#[derive(FromRow)]
struct EventRow {
    event_type: String,
    from_value: Option<String>,
    to_value: Option<String>,
    actor: Option<String>,
    at: chrono::DateTime<chrono::Utc>,
}

pub async fn unit_detail(
    State(s): State<AppState>,
    jar: SignedCookieJar,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    let u = sqlx::query_as::<_, UnitDetail>(
        "SELECT u.serial_number, u.status::text AS status, u.owner::text AS owner, \
         u.condition::text AS condition, u.verified, u.asset_tag, p.model, u.system_id \
         FROM inventory_unit u LEFT JOIN product p ON p.id = u.product_id WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?;
    let Some(u) = u else {
        return Ok(page(
            "Unit",
            user.as_deref(),
            "<h1>Unit</h1><p class=\"result err\">Not found.</p>",
        ));
    };
    let events = sqlx::query_as::<_, EventRow>(
        "SELECT event_type::text AS event_type, from_value, to_value, actor, at \
         FROM unit_event WHERE unit_id = $1 ORDER BY at, id",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    let status_opts = enum_options(&[
        "in_stock",
        "reserved",
        "in_build",
        "installed",
        "with_customer",
        "shipped",
        "rma_open",
        "pending_return",
        "defective",
        "returned",
        "scrapped",
    ]);
    let party = enum_options(&["vendor", "manufacturer"]);
    let mode = enum_options(&[
        "cec_managed",
        "customer_ships_to_cec",
        "customer_managed_assist",
    ]);

    let mut timeline = String::from(
        "<h2>Event timeline</h2><table><tr><th>When</th><th>Event</th><th>From→To</th><th>Actor</th></tr>",
    );
    for e in &events {
        timeline.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{} → {}</td><td>{}</td></tr>",
            e.at.format("%Y-%m-%d %H:%M"),
            esc(&e.event_type),
            opt(&e.from_value),
            opt(&e.to_value),
            opt(&e.actor)
        ));
    }
    timeline.push_str("</table>");

    let body = format!(
        "<h1>Unit {serial}</h1>\
<p class=\"muted\">{model} · status <code>{status}</code> · owner <code>{owner}</code> · \
condition <code>{condition}</code> · verified {verified} · tag {tag} · \
id <code>{id}</code>{sys}</p>\
<p><a href=\"/units/{id}/warranty\">warranty JSON</a> · <a href=\"/units/{id}/events\">events JSON</a> · \
<a href=\"/ui/scan/{id}\">camera verify</a></p>\
\
<h2>Change status</h2>\
<form class=\"cec\" action=\"/units/{id}/status\" data-method=\"PATCH\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>New status</label><select name=\"status\" data-required=\"1\">{status_opts}</select>\
<label>Actor</label><input name=\"actor\" value=\"{actor}\">\
<label>Note</label><input name=\"note\">\
<button>Update status</button><pre class=\"result\"></pre></form>\
\
<h2>Assign asset tag</h2>\
<form class=\"cec\" action=\"/units/{id}/asset-tag\" onsubmit=\"return cecSubmit(this)\">\
<label>Tag (optional — auto CEC-* if blank)</label><input name=\"asset_tag\">\
<button>Assign tag + get label</button><pre class=\"result\"></pre></form>\
\
<h2>Open RMA</h2>\
<form class=\"cec\" action=\"/units/{id}/rma\" onsubmit=\"return cecSubmit(this)\">\
<div class=\"row\"><div><label>Party</label><select name=\"party\"><option value=\"\">—</option>{party}</select></div>\
<div><label>Execution mode</label><select name=\"execution_mode\"><option value=\"\">—</option>{mode}</select></div></div>\
<label>Fault description</label><input name=\"fault_description\">\
<label>Actor</label><input name=\"actor\" value=\"{actor}\">\
<button>Open RMA case</button><pre class=\"result\"></pre></form>\
{timeline}",
        serial = opt(&u.serial_number),
        model = opt(&u.model),
        status = esc(&u.status),
        owner = esc(&u.owner),
        condition = esc(&u.condition),
        verified = u.verified,
        tag = opt(&u.asset_tag),
        sys = u
            .system_id
            .map(|sid| format!(" · system <a href=\"/ui/systems/{sid}\">{sid}</a>"))
            .unwrap_or_default(),
        actor = esc(user.as_deref().unwrap_or("")),
    );
    Ok(page("Unit", user.as_deref(), &body))
}

// ---------------------------------------------------------------------------
// system detail + actions
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct SystemDetail {
    label: Option<String>,
    status: String,
    validation_state: String,
    current_owner: String,
    customer_ref: Option<String>,
}

pub async fn system_detail(
    State(s): State<AppState>,
    jar: SignedCookieJar,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let user = current_user(&s, &jar).await;
    let sys = sqlx::query_as::<_, SystemDetail>(
        "SELECT label, status::text AS status, validation_state::text AS validation_state, \
         current_owner::text AS current_owner, customer_ref FROM system WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?;
    let Some(sys) = sys else {
        return Ok(page(
            "System",
            user.as_deref(),
            "<h1>System</h1><p class=\"result err\">Not found.</p>",
        ));
    };
    let members = sqlx::query_as::<_, UnitRow>(
        "SELECT u.id, u.serial_number, u.status::text AS status, u.owner::text AS owner, p.model \
         FROM inventory_unit u LEFT JOIN product p ON p.id = u.product_id \
         WHERE u.system_id = $1 ORDER BY u.intake_at",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    let mut mtable = String::from(
        "<h2>Members</h2><table><tr><th>Serial</th><th>Product</th><th>Status</th><th></th></tr>",
    );
    for m in &members {
        mtable.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td><a href=\"/ui/units/{}\">open</a></td></tr>",
            opt(&m.serial_number),
            opt(&m.model),
            esc(&m.status),
            m.id
        ));
    }
    mtable.push_str("</table>");

    let vtype = enum_options(&["eol", "post_change", "periodic", "pre_transfer", "sweep"]);
    let vresult = enum_options(&["pass", "fail"]);

    let body = format!(
        "<h1>System {label}</h1>\
<p class=\"muted\">status <code>{status}</code> · validation <code>{vstate}</code> · \
owner <code>{owner}</code>{cust} · id <code>{id}</code></p>\
<p><a href=\"/systems/{id}/asset-tag\" onclick=\"event.preventDefault();fetch('/systems/{id}/asset-tag',{{method:'POST',headers:{{'content-type':'application/json'}},body:'{{}}'}}).then(r=>r.text()).then(t=>alert(t))\">assign asset tag</a></p>\
{mtable}\
\
<h2>Add member</h2>\
<form class=\"cec\" action=\"/systems/{id}/members\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Unit ID</label><input name=\"unit_id\" data-required=\"1\" placeholder=\"uuid\">\
<label>Actor</label><input name=\"actor\" value=\"{actor}\">\
<button>Add to system</button><pre class=\"result\"></pre></form>\
\
<h2>Validate</h2>\
<form class=\"cec\" action=\"/systems/{id}/validate\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<div class=\"row\"><div><label>Type</label><select name=\"validation_type\" data-required=\"1\">{vtype}</select></div>\
<div><label>Result</label><select name=\"result\" data-required=\"1\">{vresult}</select></div></div>\
<label>Performed by</label><input name=\"performed_by\" value=\"{actor}\">\
<label>Notes</label><input name=\"notes\">\
<button>Record validation</button><pre class=\"result\"></pre></form>\
\
<h2>Deliver to customer</h2>\
<form class=\"cec\" action=\"/systems/{id}/deliver\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Customer ref</label><input name=\"customer_ref\" data-required=\"1\">\
<label>CEC warranty class</label><select name=\"cec_warranty_class\"><option value=\"full\">full</option><option value=\"refurb\">refurb</option><option value=\"none\">none</option></select>\
<label>Performed by</label><input name=\"performed_by\" value=\"{actor}\">\
<button>Deliver (starts CEC clock)</button><pre class=\"result\"></pre></form>\
\
<h2>Parts sweep</h2>\
<form class=\"cec\" action=\"/systems/{id}/sweep\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>Scanned serials (one per line or comma-separated)</label>\
<textarea name=\"scanned_serials\" data-type=\"lines\" data-required=\"1\" rows=\"4\"></textarea>\
<label>Performed by</label><input name=\"performed_by\" value=\"{actor}\">\
<button>Run sweep</button><pre class=\"result\"></pre></form>\
\
<h2>Transfer ownership</h2>\
<form class=\"cec\" action=\"/systems/{id}/transfer\" data-reload=\"1\" onsubmit=\"return cecSubmit(this)\">\
<label>To owner ref</label><input name=\"to_owner_ref\" data-required=\"1\">\
<label>Authorizing sweep id (optional; else must be validated)</label><input name=\"sweep_id\">\
<label>Performed by</label><input name=\"performed_by\" value=\"{actor}\">\
<button>Transfer</button><pre class=\"result\"></pre></form>",
        label = opt(&sys.label),
        status = esc(&sys.status),
        vstate = esc(&sys.validation_state),
        owner = esc(&sys.current_owner),
        cust = sys
            .customer_ref
            .as_deref()
            .map(|c| format!(" · customer <code>{}</code>", esc(c)))
            .unwrap_or_default(),
        actor = esc(user.as_deref().unwrap_or("")),
    );
    Ok(page("System", user.as_deref(), &body))
}

// ---------------------------------------------------------------------------
// scan island (scope §13.1)
// ---------------------------------------------------------------------------

/// Camera + barcode scan island. Uses native BarcodeDetector where present; a WASM fallback
/// (zxing-wasm) is wired for Safari/iOS in a later pass. Needs a secure context. Posts the
/// scanned serial to `POST /units/{id}/verify`.
pub async fn scan_page(
    State(s): State<AppState>,
    jar: SignedCookieJar,
    Path(unit_id): Path<Uuid>,
) -> Html<String> {
    let user = current_user(&s, &jar).await;
    let body = format!(
        "<h1>Scan to verify</h1><p class=\"l\">Unit <code>{unit_id}</code> · \
<a href=\"/ui/units/{unit_id}\">unit detail</a></p>\
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
    page("Scan", user.as_deref(), &body)
}

/// Landing scan page without a target unit, explaining the island.
pub async fn scan_index(State(s): State<AppState>, jar: SignedCookieJar) -> Html<String> {
    let user = current_user(&s, &jar).await;
    page(
        "Scan",
        user.as_deref(),
        "<h1>Scan</h1><p class=\"l\">Open a unit (<a href=\"/ui/units\">Units</a>) and choose \
<b>camera verify</b>, or go to <code>/ui/scan/&lt;unit-id&gt;</code> on a phone (HTTPS or \
localhost) to verify a unit's serial with the camera (scope §13.1). Long-receipt guided capture \
and the WASM fallback for Safari/iOS land in a later pass.</p>",
    )
}
