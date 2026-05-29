use imfwizard_core::timeline::{self, CplInfo, SegmentEntry};
use std::path::Path;

#[tauri::command]
pub fn list_cpls(imp_dir: String) -> Result<Vec<CplInfo>, String> {
    let path = Path::new(&imp_dir);
    if !path.exists() {
        return Err("IMP directory not found".into());
    }
    Ok(timeline::list_cpls(path))
}

#[tauri::command]
pub fn get_timeline(cpl_path: String) -> Result<Vec<SegmentEntry>, String> {
    let path = Path::new(&cpl_path);
    if !path.exists() {
        return Err("CPL file not found".into());
    }
    Ok(timeline::get_timeline(path))
}
