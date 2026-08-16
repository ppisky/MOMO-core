use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use momo_core::{
    ExportSelection, MomoCore, export_moc,
    momo_domain::{CharacterCard, Conversation, Message, MessageRole},
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests/fixtures/dmw-memory-guaranteed.moc"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let directory = tempfile::tempdir()?;
    let core = MomoCore::initialize(directory.path()).await?;
    let scope_id = Uuid::parse_str("019c0000-0000-7000-8000-000000000101")?;
    let character_id = Uuid::parse_str("019c0000-0000-7000-8000-000000000102")?;
    let conversation_id = Uuid::parse_str("019c0000-0000-7000-8000-000000000103")?;
    let created_at = Utc
        .with_ymd_and_hms(2026, 7, 30, 12, 0, 0)
        .single()
        .ok_or("invalid fixture timestamp")?;

    core.store()
        .save_character(&CharacterCard {
            id: character_id,
            scope_id,
            name: "艾琳".to_owned(),
            version: "2.0.0".to_owned(),
            author_name: "MOMO Test Fixture".to_owned(),
            author_url: None,
            character_markdown: "# 艾琳\n\n艾琳只把对话中明确确认的事实视为长期事实。".to_owned(),
            user_markdown: "# 用户\n\n用户在对话中会明确说明关系、性质与持续状态。".to_owned(),
            opening_markdown: None,
            created_at,
            updated_at: created_at,
        })
        .await?;
    core.store()
        .save_conversation(&Conversation {
            id: conversation_id,
            scope_id,
            character_id: Some(character_id),
            title: "Dual-Mem 明确关系测试".to_owned(),
            created_at,
            updated_at: created_at,
        })
        .await?;

    let turns = [
        (
            MessageRole::User,
            "请明确记住：我的名字是林澈。艾琳和林澈已经正式确认恋人关系，这不是猜测，也不是临时扮演。",
        ),
        (
            MessageRole::Assistant,
            "我确认：艾琳与林澈是双方明确承认的恋人，这是一段持续关系。",
        ),
        (
            MessageRole::User,
            "再确认两个长期性质：林澈对花生严重过敏；艾琳害怕雷声。两项都是真实、稳定、以后会影响行为的设定。",
        ),
        (
            MessageRole::Assistant,
            "已确认。涉及食物时必须避开花生；雷雨场景中，艾琳会因雷声明显紧张并主动靠近林澈。",
        ),
        (
            MessageRole::User,
            "今天发生的重要事件：我们在车站约定，无论争吵多严重都不突然失联，必须先说明自己需要冷静多久。",
        ),
        (
            MessageRole::Assistant,
            "这个车站约定已经由双方明确同意，会持续约束以后处理冲突的方式。",
        ),
    ];
    for (index, (role, content)) in turns.into_iter().enumerate() {
        core.store()
            .append_message(&Message {
                id: Uuid::from_u128(0x019c_0000_0000_7000_8000_0000_0000_0200 + index as u128),
                conversation_id,
                role,
                content: content.to_owned(),
                created_at: created_at + chrono::Duration::seconds(index as i64),
            })
            .await?;
    }

    export_moc(
        &core,
        &output,
        scope_id,
        &serde_json::json!({"schema_version": 2}),
        ExportSelection {
            config: false,
            characters: true,
            conversations: true,
            memory: true,
            semantic_graph: true,
            character_id: None,
        },
    )
    .await?;
    println!("{}", output.display());
    Ok(())
}
