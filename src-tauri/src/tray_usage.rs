use crate::account_usage::{AccountUsageMetric, AccountUsageProviderState, AccountUsageSnapshot};
#[cfg(target_os = "macos")]
const PROVIDER_ATTACHMENT_SIZE: f64 = 16.0;
#[cfg(target_os = "macos")]
const PROVIDER_ATTACHMENT_Y_OFFSET: f64 = -3.4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MenuBarUsageItem {
    provider_id: String,
    usage_title: String,
}

impl MenuBarUsageItem {
    pub(crate) fn usage_title(&self) -> &str {
        &self.usage_title
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn account_usage_percentage_suffix(conn: &rusqlite::Connection) -> Option<String> {
    let items = account_usage_menu_bar_items(conn);
    if items.is_empty() {
        None
    } else {
        Some(
            items
                .iter()
                .map(|item| item.usage_title())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

pub(crate) fn account_usage_menu_bar_items(conn: &rusqlite::Connection) -> Vec<MenuBarUsageItem> {
    let states = match crate::account_usage::store::list_provider_states(conn) {
        Ok(states) => states,
        Err(_) => return Vec::new(),
    };
    let snapshots = crate::account_usage::store::latest_snapshots(conn).unwrap_or_default();
    build_account_usage_menu_bar_items(&states, &snapshots)
}

#[cfg(test)]
pub(crate) fn build_account_usage_percentage_suffix(
    states: &[AccountUsageProviderState],
    snapshots: &[AccountUsageSnapshot],
) -> Option<String> {
    let items = build_account_usage_menu_bar_items(states, snapshots);
    if items.is_empty() {
        None
    } else {
        Some(
            items
                .iter()
                .map(|item| item.usage_title())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

pub(crate) fn build_account_usage_menu_bar_items(
    states: &[AccountUsageProviderState],
    snapshots: &[AccountUsageSnapshot],
) -> Vec<MenuBarUsageItem> {
    states
        .iter()
        .filter(|state| state.enabled && state.show_in_menu_bar)
        .filter(|state| provider_icon_available(&state.provider_id))
        .map(|state| MenuBarUsageItem {
            provider_id: state.provider_id.clone(),
            usage_title: provider_usage_title(&state.provider_id, snapshots),
        })
        .collect()
}

fn provider_icon_available(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "codex" | "claude-code" | "cursor" | "github-copilot"
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn set_main_tray_usage_title(
    tray: &tauri::tray::TrayIcon,
    token_title: String,
    items: Vec<MenuBarUsageItem>,
) -> tauri::Result<()> {
    let (plain_title, markers) = build_title_with_icon_markers(&token_title, &items);

    tray.with_inner_tray_icon(move |inner| {
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_foundation::NSString;
        use objc2_foundation::{NSMutableAttributedString, NSRange};

        let Some(status_item) = inner.ns_status_item() else {
            return;
        };
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(button) = status_item.button(mtm) else {
            return;
        };

        button.setTitle(&NSString::from_str(&plain_title));
        let attributed_title = button.attributedTitle();
        let title_color = attributed_title_color(&attributed_title);
        let mutable_title = NSMutableAttributedString::initWithAttributedString(
            NSMutableAttributedString::alloc(),
            &attributed_title,
        );

        for (location, provider_id) in markers.into_iter().rev() {
            if let Some(attachment) =
                provider_attachment_attributed_string(&provider_id, &title_color)
            {
                mutable_title.replaceCharactersInRange_withAttributedString(
                    NSRange::new(location, 1),
                    &attachment,
                );
            }
        }

        button.setAttributedTitle(&mutable_title);

        // 同步 Tauri 覆盖在 status item 上的事件层尺寸，避免标题宽度变化后点击区域滞后。
        let frame = button.frame();
        for view in button.subviews().iter() {
            view.setFrame(frame);
        }
    })
}

#[cfg(target_os = "macos")]
fn provider_attachment_attributed_string(
    provider_id: &str,
    color: &objc2_app_kit::NSColor,
) -> Option<objc2::rc::Retained<objc2_foundation::NSAttributedString>> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSAttributedStringAttachmentConveniences, NSImage, NSTextAttachment};
    use objc2_foundation::{NSAttributedString, NSData, NSPoint, NSRect, NSSize};

    let data = NSData::with_bytes(provider_icon_bytes(provider_id)?);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    let image = tinted_provider_icon(&image, color);

    let attachment = NSTextAttachment::new();
    attachment.setImage(Some(&image));
    attachment.setBounds(NSRect::new(
        NSPoint::new(0.0, PROVIDER_ATTACHMENT_Y_OFFSET),
        NSSize::new(PROVIDER_ATTACHMENT_SIZE, PROVIDER_ATTACHMENT_SIZE),
    ));

    Some(NSAttributedString::attributedStringWithAttachment(
        &attachment,
    ))
}

#[cfg(target_os = "macos")]
fn attributed_title_color(
    title: &objc2_foundation::NSAttributedString,
) -> objc2::rc::Retained<objc2_app_kit::NSColor> {
    use core::ptr;

    use objc2_app_kit::{NSColor, NSForegroundColorAttributeName};

    unsafe {
        title
            .attribute_atIndex_effectiveRange(NSForegroundColorAttributeName, 0, ptr::null_mut())
            .and_then(|value| value.downcast::<NSColor>().ok())
            .unwrap_or_else(NSColor::labelColor)
    }
}

#[cfg(target_os = "macos")]
fn tinted_provider_icon(
    source: &objc2_app_kit::NSImage,
    color: &objc2_app_kit::NSColor,
) -> objc2::rc::Retained<objc2_app_kit::NSImage> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSCompositingOperation, NSImage, NSRectFillUsingOperation};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let size = NSSize::new(PROVIDER_ATTACHMENT_SIZE, PROVIDER_ATTACHMENT_SIZE);
    let rect = NSRect::new(NSPoint::ZERO, size);
    let image = NSImage::initWithSize(NSImage::alloc(), size);

    #[allow(deprecated)]
    {
        image.lockFocus();
        source.drawInRect_fromRect_operation_fraction(
            rect,
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.0,
        );
        color.setFill();
        NSRectFillUsingOperation(rect, NSCompositingOperation::SourceIn);
        image.unlockFocus();
    }
    image.setTemplate(false);

    image
}

#[cfg(target_os = "macos")]
fn build_title_with_icon_markers(
    token_title: &str,
    items: &[MenuBarUsageItem],
) -> (String, Vec<(usize, String)>) {
    let mut title = token_title.to_string();
    let mut utf16_len = title.encode_utf16().count();
    let mut markers = Vec::with_capacity(items.len());

    for item in items {
        title.push(' ');
        utf16_len += 1;
        markers.push((utf16_len, item.provider_id.clone()));
        title.push('\u{fffc}');
        utf16_len += 1;
        title.push(' ');
        utf16_len += 1;
        title.push_str(&item.usage_title);
        utf16_len += item.usage_title.encode_utf16().count();
    }

    (title, markers)
}

#[cfg(target_os = "macos")]
fn provider_icon_bytes(provider_id: &str) -> Option<&'static [u8]> {
    match provider_id {
        "codex" => Some(include_bytes!("../icons/provider-menubar/codex.pdf")),
        "claude-code" => Some(include_bytes!("../icons/provider-menubar/claude-code.pdf")),
        "cursor" => Some(include_bytes!("../icons/provider-menubar/cursor.pdf")),
        "github-copilot" => Some(include_bytes!(
            "../icons/provider-menubar/github-copilot.pdf"
        )),
        _ => None,
    }
}

fn provider_usage_title(provider_id: &str, snapshots: &[AccountUsageSnapshot]) -> String {
    best_provider_percentage(provider_id, snapshots)
        .map(|percentage| format!("{:.0}%", percentage.round()))
        .unwrap_or_else(|| "--%".to_string())
}

fn best_provider_percentage(provider_id: &str, snapshots: &[AccountUsageSnapshot]) -> Option<f64> {
    snapshots
        .iter()
        .filter(|snapshot| snapshot.provider_id == provider_id)
        .flat_map(|snapshot| snapshot.metrics.iter())
        .filter_map(metric_percentage)
        .max_by(|a, b| a.total_cmp(b))
}

fn metric_percentage(metric: &AccountUsageMetric) -> Option<f64> {
    let percentage = metric
        .percentage
        .or_else(|| match (metric.used, metric.limit) {
            (Some(used), Some(limit)) if limit > 0.0 => Some(used / limit * 100.0),
            _ => None,
        })?;
    if percentage.is_finite() {
        Some(percentage.clamp(0.0, 100.0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_usage::{
        AccountUsageConfidence, AccountUsageMetricScope, AccountUsageSource, AccountUsageStatus,
    };

    #[test]
    fn test_provider_usage_title_uses_highest_percentage() {
        let snapshots = vec![AccountUsageSnapshot {
            provider_id: "codex".to_string(),
            account_key: "default".to_string(),
            account_label: None,
            plan: None,
            status: AccountUsageStatus::Ok,
            source: AccountUsageSource::AuthFile,
            confidence: AccountUsageConfidence::High,
            observed_at: "2026-05-16T00:00:00Z".to_string(),
            period_start: None,
            period_end: None,
            reset_at: None,
            stale: false,
            error: None,
            metrics: vec![
                AccountUsageMetric {
                    metric_key: "codex.primary".to_string(),
                    label: "5h".to_string(),
                    unit: "percent".to_string(),
                    scope: AccountUsageMetricScope::Workspace,
                    used: Some(20.0),
                    limit: Some(100.0),
                    remaining: Some(80.0),
                    percentage: Some(20.0),
                    reset_at: None,
                },
                AccountUsageMetric {
                    metric_key: "codex.secondary".to_string(),
                    label: "7d".to_string(),
                    unit: "percent".to_string(),
                    scope: AccountUsageMetricScope::Workspace,
                    used: Some(42.0),
                    limit: Some(100.0),
                    remaining: Some(58.0),
                    percentage: Some(42.0),
                    reset_at: None,
                },
            ],
        }];

        assert_eq!(provider_usage_title("codex", &snapshots), "42%");
    }

    #[test]
    fn test_build_account_usage_percentage_suffix() {
        let states = vec![
            AccountUsageProviderState {
                provider_id: "codex".to_string(),
                enabled: true,
                show_in_menu_bar: true,
                refresh_interval_secs: 300,
                last_refresh_at: None,
                retry_after_until: None,
                credential_ref: None,
                credential_label: None,
                auto_discovery_enabled: false,
            },
            AccountUsageProviderState {
                provider_id: "cursor".to_string(),
                enabled: false,
                show_in_menu_bar: true,
                refresh_interval_secs: 600,
                last_refresh_at: None,
                retry_after_until: None,
                credential_ref: None,
                credential_label: None,
                auto_discovery_enabled: false,
            },
        ];
        let snapshots = vec![AccountUsageSnapshot {
            provider_id: "codex".to_string(),
            account_key: "default".to_string(),
            account_label: None,
            plan: None,
            status: AccountUsageStatus::Ok,
            source: AccountUsageSource::AuthFile,
            confidence: AccountUsageConfidence::High,
            observed_at: "2026-05-16T00:00:00Z".to_string(),
            period_start: None,
            period_end: None,
            reset_at: None,
            stale: false,
            error: None,
            metrics: vec![AccountUsageMetric {
                metric_key: "codex.primary".to_string(),
                label: "5h".to_string(),
                unit: "percent".to_string(),
                scope: AccountUsageMetricScope::Workspace,
                used: Some(35.0),
                limit: Some(100.0),
                remaining: Some(65.0),
                percentage: Some(35.0),
                reset_at: None,
            }],
        }];

        assert_eq!(
            build_account_usage_percentage_suffix(&states, &snapshots),
            Some("35%".to_string())
        );
    }

    #[test]
    fn test_build_account_usage_percentage_suffix_filters_unknown_provider() {
        let states = vec![AccountUsageProviderState {
            provider_id: "unknown".to_string(),
            enabled: true,
            show_in_menu_bar: true,
            refresh_interval_secs: 300,
            last_refresh_at: None,
            retry_after_until: None,
            credential_ref: None,
            credential_label: None,
            auto_discovery_enabled: false,
        }];

        assert_eq!(build_account_usage_percentage_suffix(&states, &[]), None);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_build_title_with_icon_markers_places_icon_after_token_title() {
        let items = vec![MenuBarUsageItem {
            provider_id: "codex".to_string(),
            usage_title: "18%".to_string(),
        }];
        let (title, markers) = build_title_with_icon_markers("19.7M", &items);

        assert_eq!(title, "19.7M \u{fffc} 18%");
        assert_eq!(markers, vec![(6, "codex".to_string())]);
    }
}
