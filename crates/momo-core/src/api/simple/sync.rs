//! Synchronization cursors, conflicts, tombstones, and outbox state.

use super::*;

pub async fn pending_outbox_json(limit: u32) -> Result<String, String> {
    let _ = limit;
    Ok("[]".to_owned())
}

pub async fn pending_outbox_for_categories_json(
    categories_json: String,
    limit: u32,
) -> Result<String, String> {
    let _ = (categories_json, limit);
    Ok("[]".to_owned())
}

pub async fn outbox_count() -> Result<i64, String> {
    Ok(0)
}

pub async fn local_tombstone_ids_json(object_type: String) -> Result<String, String> {
    let ids = core()?
        .store()
        .tombstone_ids(&object_type)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&ids).map_err(|error| error.to_string())
}

pub async fn recently_deleted_json(limit: u32) -> Result<String, String> {
    let items = core()?
        .store()
        .recently_deleted(limit)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&items).map_err(|error| error.to_string())
}

pub async fn restore_recently_deleted(
    object_type: String,
    object_id: String,
) -> Result<bool, String> {
    let object_id = uuid::Uuid::parse_str(&object_id).map_err(|error| error.to_string())?;
    let store = core()?.store();
    let restored = store
        .restore_recently_deleted(&object_type, object_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(restored)
}

pub async fn purge_recently_deleted(
    object_type: String,
    object_id: String,
) -> Result<bool, String> {
    let object_id = uuid::Uuid::parse_str(&object_id).map_err(|error| error.to_string())?;
    let store = core()?.store();
    let purged = store
        .forget_recently_deleted(&format!("local_only:{object_type}"), object_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(purged)
}

pub async fn forget_recently_deleted(
    object_type: String,
    object_id: String,
) -> Result<bool, String> {
    let object_id = uuid::Uuid::parse_str(&object_id).map_err(|error| error.to_string())?;
    let store = core()?.store();
    let forgotten = store
        .forget_recently_deleted(&format!("local_only:{object_type}"), object_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(forgotten)
}

pub async fn sync_cursor(remote: String) -> Result<i64, String> {
    let _ = remote;
    Ok(0)
}

pub async fn sync_revision(remote: String, object_id: String) -> Result<Option<i64>, String> {
    let _ = (remote, object_id);
    Ok(None)
}

pub async fn backfill_sync_outbox_json(
    scope_id: String,
    remote: String,
    allowed_categories_json: String,
) -> Result<String, String> {
    let _ = (scope_id, remote, allowed_categories_json);
    Err("同步已禁用：当前 MOMO Core 只生成本地数据和 .moc".to_owned())
}

pub async fn apply_remote_sync_objects_json(
    remote: String,
    objects_json: String,
) -> Result<String, String> {
    let _ = (remote, objects_json);
    Err("同步已禁用：当前 MOMO Core 不接收远端同步对象".to_owned())
}

pub async fn apply_remote_sync_objects_filtered_json(
    remote: String,
    objects_json: String,
    allowed_categories_json: String,
) -> Result<String, String> {
    let _ = (remote, objects_json, allowed_categories_json);
    Err("同步已禁用：当前 MOMO Core 不接收远端同步对象".to_owned())
}

pub async fn restore_cloud_deleted_object_json(
    remote: String,
    object_json: String,
) -> Result<bool, String> {
    let _ = (remote, object_json);
    Err("同步已禁用：当前 MOMO Core 没有云端删除恢复入口".to_owned())
}

pub async fn record_sync_publish_json(remote: String, object_json: String) -> Result<(), String> {
    let _ = (remote, object_json);
    Err("同步已禁用：当前 MOMO Core 没有发布确认入口".to_owned())
}

pub async fn sync_conflicts_json(remote: String) -> Result<String, String> {
    let _ = remote;
    Ok("[]".to_owned())
}

pub async fn resolve_sync_conflict(
    remote: String,
    object_id: String,
    keep_local: bool,
) -> Result<(), String> {
    let _ = (remote, object_id, keep_local);
    Err("同步已禁用：当前 MOMO Core 没有同步冲突入口".to_owned())
}

pub async fn acknowledge_outbox(id: String) -> Result<bool, String> {
    let _ = id;
    Ok(false)
}

pub async fn fail_outbox(id: String, error: String) -> Result<bool, String> {
    let _ = (id, error);
    Ok(false)
}
