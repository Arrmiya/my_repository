// Tauri v2 浏览器 + 视频嗅探下载 v0.3

use futures_util::StreamExt;
use reqwest::Client;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{command, AppHandle, Emitter, Manager};
use tokio::sync::Mutex;
use url::Url;

// ─── JS 初始化脚本（每页加载前运行） ───

const INIT_SCRIPT: &str = r#"
(function(){
if(window.__bInjected)return;window.__bInjected=1;

// ━━ 阶段 1：立即运行 —— Scheme 拦截 + 网络拦截 ━━

var ALLOWED=/^(http|https|javascript|mailto|tel|about|data|blob|#|asset|tauri):/i;
function blocked(u){
  if(!u||typeof u!=='string')return false;
  var s=u.trim().split(':')[0];
  return s&&!ALLOWED.test(s+':');
}

// location.href setter —— 百度/B站用这个跳转
(function(){
  var d=Object.getOwnPropertyDescriptor(Location.prototype,'href');
  if(d&&d.set){
    var _s=d.set;
    Object.defineProperty(Location.prototype,'href',{
      get:d.get,configurable:true,
      set:function(v){if(blocked(v)){console.log('[Blk href]',v.split(':')[0]);return;}_s.call(this,v);}
    });
  }
})();
// location.assign / replace
try{var _a=Location.prototype.assign;Location.prototype.assign=function(v){if(blocked(v)){console.log('[Blk assign]',v.split(':')[0]);return;}_a.call(this,v);};}catch(e){}
try{var _r=Location.prototype.replace;Location.prototype.replace=function(v){if(blocked(v)){console.log('[Blk replace]',v.split(':')[0]);return;}_r.call(this,v);};}catch(e){}
// window.open
(function(){var _ow=window.open;window.open=function(u){if(typeof u==='string'&&blocked(u))return null;return _ow.apply(this,arguments);};})();

// —— 视频/音频检测引擎 ——
var detected=new Set();
function notify(url,type,title){
  var k=url+'|'+type;if(detected.has(k))return;detected.add(k);
  try{window.__TAURI__&&window.__TAURI__.invoke('detect_media',{url:url,mediaType:type,pageTitle:title||document.title||''});}catch(e){}
}

// 拦截所有 fetch 请求
var MEDIA_RE=/\.(m3u8|mp4|ts|m4s|m4a|m4v|mkv|webm|flv|mov|avi|wmv|mp3|aac|ogg|wav|3gp|ogv|f4v)(\?|$)/i;
var _fetch=window.fetch;
window.fetch=function(url,opts){
  var u=typeof url==='string'?url:(url instanceof Request?url.url:'');
  if(MEDIA_RE.test(u))notify(u,'fetch',document.title);
  return _fetch.apply(this,arguments);
};
// 拦截所有 XHR 请求
var _open=XMLHttpRequest.prototype.open;
XMLHttpRequest.prototype.open=function(m,u){
  if(MEDIA_RE.test(u))notify(u,'xhr',document.title);
  return _open.apply(this,arguments);
};

// ━━ 阶段 2：DOM 就绪后 —— UI + 扫描 ━━

function onDOM(){
  // 状态栏安全区
  var sb=document.createElement('div');
  sb.id='__sb';sb.style.cssText='position:fixed;top:0;left:0;right:0;height:36px;z-index:2147483646;pointer-events:none;background:rgba(0,0,0,0.03);';
  document.documentElement.appendChild(sb);
  document.body.style.paddingTop='36px';

  // <a> 点击拦截
  document.addEventListener('click',function(e){
    var a=e.target.closest('a');if(!a||!a.href)return;
    var raw=a.getAttribute('href')||'';if(!raw||raw.startsWith('#'))return;
    if(blocked(a.href)){e.preventDefault();e.stopImmediatePropagation();console.log('[Blk click]',a.href.split(':')[0]);}
  },true);

  // 视频元素扫描（包括 blob: 地址）
  function scan(){
    document.querySelectorAll('video,audio').forEach(function(el){
      if(el.src)notify(el.src,el.tagName.toLowerCase(),document.title);
      el.querySelectorAll('source').forEach(function(s){if(s.src)notify(s.src,el.tagName.toLowerCase(),document.title);});
    });
  }
  scan();
  new MutationObserver(scan).observe(document.documentElement,{childList:true,subtree:true});

  // 浮动按钮组 —— 🏠 返回 + ⬇ 下载
  var ui=document.createElement('div');
  ui.id='__browser_float';
  ui.style.cssText='position:fixed;bottom:24px;right:16px;z-index:2147483647;display:flex;flex-direction:column;gap:10px;align-items:flex-end;';
  ui.innerHTML=
    '<button id="__b_home" style="background:#6b4ff7;color:#fff;border:none;border-radius:24px;padding:10px 18px;font-size:15px;font-weight:600;box-shadow:0 4px 12px rgba(107,79,247,0.4);cursor:pointer;">🏠 返回首页</button>'+
    '<button id="__b_dl" style="display:none;background:#fff;color:#1d1d1f;border:1px solid #6b4ff7;border-radius:24px;padding:10px 18px;font-size:14px;font-weight:600;box-shadow:0 4px 12px rgba(0,0,0,0.15);cursor:pointer;"></button>';
  document.body.appendChild(ui);

  document.getElementById('__b_home').onclick=function(){
    try{window.__TAURI__&&window.__TAURI__.invoke('go_home');}catch(e){}
  };

  // 监听下载事件
  try{
    window.__TAURI__&&window.__TAURI__.event.listen('media-detected',function(ev){
      var p=ev.payload,btn=document.getElementById('__b_dl');
      btn.style.display='block';
      btn.textContent='⬇ 下载 '+p.filename;
      btn.title=p.url;
      btn.onclick=function(){
        try{window.__TAURI__&&window.__TAURI__.invoke('start_download',{url:p.url,filename:p.filename});}catch(e){}
        btn.textContent='⏳ 下载中...';
        btn.style.background='#f0f0f0';
      };
    });
    window.__TAURI__&&window.__TAURI__.event.listen('download-progress',function(ev){
      var p=ev.payload,btn=document.getElementById('__b_dl');
      if(p.done){
        btn.textContent='✅ 下载完成';
        btn.style.background='#e8f5e9';
      }else{
        var mb=(p.downloaded/1048576).toFixed(1);
        var t=p.total>0?(p.total/1048576).toFixed(1):'?';
        btn.textContent='⏳ '+mb+'/'+t+' MB';
      }
    });
  }catch(e){}
  console.log('[Browser] ready — scheme blocker + video detector active');
}
if(document.readyState==='loading'){document.addEventListener('DOMContentLoaded',onDOM);}else{onDOM();}
})();
"#;

// ─── 数据结构 ───

#[derive(Clone, Serialize)]
struct MediaInfo {
    url: String,
    media_type: String,
    page_title: String,
    filename: String,
    size: String,
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    url: String,
    filename: String,
    downloaded: u64,
    total: u64,
    done: bool,
    path: String,
}

struct AppState {
    cancel_flag: Arc<AtomicBool>,
}

// ─── 命令 ───

#[command]
async fn navigate(app: AppHandle, url: String) -> Result<(), String> {
    let webview = app.get_webview_window("main").ok_or("未找到主窗口")?;
    let target_url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else if url.contains('.') && !url.contains(' ') {
        format!("https://{}", url)
    } else {
        format!("https://www.google.com/search?q={}", url.replace(' ', "+"))
    };
    let parsed = Url::parse(&target_url).map_err(|e| format!("无效 URL: {}", e))?;
    webview.navigate(parsed).map_err(|e| format!("导航失败: {}", e))?;
    Ok(())
}

#[command]
async fn go_home(app: AppHandle) -> Result<(), String> {
    let webview = app.get_webview_window("main").ok_or("未找到主窗口")?;
    let _ = webview.eval("location.href = '/'");
    Ok(())
}

#[command]
async fn detect_media(
    app: AppHandle, url: String, media_type: String, page_title: String,
) -> Result<(), String> {
    let filename = extract_filename(&url, &media_type);
    app.emit("media-detected", &MediaInfo {
        url: url.clone(), media_type, page_title, filename: filename.clone(), size: "未知".into(),
    }).map_err(|e| format!("事件发送失败: {}", e))?;
    println!("[Browser] 检测到媒体: {} ({})", filename, url);
    Ok(())
}

#[command]
async fn start_download(
    app: AppHandle, state: tauri::State<'_, Arc<Mutex<AppState>>>,
    url: String, filename: String,
) -> Result<String, String> {
    let s = state.lock().await;
    s.cancel_flag.store(false, Ordering::SeqCst);
    let cancel = s.cancel_flag.clone();
    drop(s);

    let download_dir = get_download_dir(&app)?;
    let fname = sanitize_filename(&filename);
    let file_path = download_dir.join(&fname);

    let app_handle = app.clone();
    let url_clone = url.clone();
    let f_dl = fname.clone();
    let f_ok = fname.clone();
    let f_err = fname.clone();

    tokio::spawn(async move {
        match download_file(&url_clone, &file_path, &f_dl, &app_handle, &cancel).await {
            Ok(path) => {
                let _ = app_handle.emit("download-progress", &DownloadProgress {
                    url: url_clone, filename: f_ok, downloaded: 0, total: 0, done: true, path,
                });
            }
            Err(e) => {
                let _ = app_handle.emit("download-progress", &DownloadProgress {
                    url: url_clone, filename: f_err, downloaded: 0, total: 0, done: true,
                    path: format!("错误: {}", e),
                });
            }
        }
    });
    Ok(format!("开始下载: {}", fname))
}

async fn download_file(
    url: &str, file_path: &PathBuf, _filename: &str, app: &AppHandle, cancel: &AtomicBool,
) -> Result<String, String> {
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目录: {}", e))?;
    }
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36")
        .build().map_err(|e| format!("HTTP 客户端失败: {}", e))?;
    let resp = client.get(url).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let total = resp.content_length().unwrap_or(0);
    let mut file = fs::File::create(file_path).map_err(|e| format!("无法创建文件: {}", e))?;
    let mut dl: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) { let _ = fs::remove_file(file_path); return Err("已取消".into()); }
        let chunk = chunk.map_err(|e| format!("读取失败: {}", e))?;
        file.write_all(&chunk).map_err(|e| format!("写入失败: {}", e))?;
        dl += chunk.len() as u64;
        let _ = app.emit("download-progress", &DownloadProgress {
            url: url.to_string(), filename: file_path.to_string_lossy().to_string(),
            downloaded: dl, total, done: false, path: file_path.to_string_lossy().to_string(),
        });
    }
    file.flush().map_err(|e| format!("刷新失败: {}", e))?;
    Ok(file_path.to_string_lossy().to_string())
}

fn get_download_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(d) = app.path().download_dir() { fs::create_dir_all(&d).ok(); return Ok(d); }
    let d = app.path().app_data_dir().map(|p| p.join("Downloads")).map_err(|e| format!("{}", e))?;
    fs::create_dir_all(&d).ok(); Ok(d)
}

fn extract_filename(url: &str, media_type: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("video");
    if name.contains('.') { name.to_string() } else {
        let ext = match media_type { "audio" => "mp3", _ => "mp4" };
        format!("{}_{}.{}", name, rand_suffix(), ext)
    }
}

fn rand_suffix() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_micros()).unwrap_or(0)
}

fn sanitize_filename(name: &str) -> String {
    name.chars().map(|c| match c { '/'|'\\'|':'|'*'|'?'|'"'|'<'|'>'|'|'|'\n'|'\r' => '_', _ => c }).collect()
}

// ─── 入口 ───

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(AppState { cancel_flag: Arc::new(AtomicBool::new(false)) })))
        .setup(move |app| {
            if let Some(old) = app.get_webview_window("main") { let _ = old.close(); }
            let _ = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
                .title("Tauri 浏览器")
                .inner_size(390.0, 844.0)
                .initialization_script(INIT_SCRIPT)
                .build()
                .expect("创建主窗口失败");
            println!("[Browser] v0.3 已启动");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![navigate, go_home, detect_media, start_download])
        .run(tauri::generate_context!())
        .expect("启动应用时出错");
}
