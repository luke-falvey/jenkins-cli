//! Human-readable text output.

use std::fmt::Write as _;
use std::time::{Duration, UNIX_EPOCH};

use crate::models::{
    Build, BuildSummary, Computer, ComputerSet, Controller, ItemRef, Job, PluginManager, Queue,
    QueueItem, View, WhoAmI, color_status, item_kind,
};

/// A plain text table with left-aligned columns.
pub struct Table {
    headers: Vec<&'static str>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&'static str]) -> Self {
        Self {
            headers: headers.to_vec(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        debug_assert_eq!(cells.len(), self.headers.len());
        self.rows.push(cells);
    }

    pub fn render(&self) -> String {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let mut out = String::new();
        let header_cells: Vec<String> = self.headers.iter().map(|h| h.to_string()).collect();
        for cells in std::iter::once(&header_cells).chain(&self.rows) {
            let mut line = String::new();
            for (i, (cell, width)) in cells.iter().zip(&widths).enumerate() {
                if i + 1 == cells.len() {
                    line.push_str(cell);
                } else {
                    let _ = write!(line, "{cell:<width$}  ");
                }
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

/// Formats milliseconds since the epoch as an RFC 3339 UTC time.
pub fn format_timestamp(millis: i64) -> String {
    match u64::try_from(millis) {
        Ok(ms) if ms > 0 => {
            humantime::format_rfc3339_seconds(UNIX_EPOCH + Duration::from_millis(ms)).to_string()
        }
        _ => "-".to_string(),
    }
}

/// Formats a duration in milliseconds as `1h 02m`, `3m 05s` or `12s`.
pub fn format_duration(millis: i64) -> String {
    let secs = millis.max(0) / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else if millis > 0 && secs == 0 {
        "<1s".to_string()
    } else {
        format!("{s}s")
    }
}

fn or_dash(value: Option<&str>) -> &str {
    match value {
        Some(v) if !v.trim().is_empty() => v,
        _ => "-",
    }
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_string()
}

/// Key/value lines with the values lined up.
fn details(pairs: &[(&str, String)]) -> String {
    let width = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0) + 1;
    pairs
        .iter()
        .map(|(k, v)| format!("{:<width$} {v}\n", format!("{k}:")))
        .collect()
}

fn build_result(result: Option<&str>, building: bool) -> String {
    if building {
        "BUILDING".to_string()
    } else {
        or_dash(result).to_string()
    }
}

pub fn who_am_i(me: &WhoAmI) -> String {
    let state = if me.anonymous {
        "anonymous"
    } else if me.authenticated {
        "authenticated"
    } else {
        "not authenticated"
    };
    details(&[
        ("User", me.name.clone()),
        ("Status", state.to_string()),
        (
            "Authorities",
            or_dash(Some(&me.authorities.join(", "))).to_string(),
        ),
    ])
}

pub fn controller(c: &Controller) -> String {
    details(&[
        ("URL", or_dash(Some(&c.url)).to_string()),
        ("Description", or_dash(c.description.as_deref()).to_string()),
        ("Mode", or_dash(Some(&c.mode)).to_string()),
        ("Executors", c.num_executors.to_string()),
        (
            "Security",
            if c.use_security {
                "enabled"
            } else {
                "disabled"
            }
            .to_string(),
        ),
        ("Quieting down", yes_no(c.quieting_down)),
        ("Jobs", c.jobs.len().to_string()),
        ("Views", c.views.len().to_string()),
        (
            "Primary view",
            or_dash(c.primary_view.as_ref().map(|v| v.name.as_str())).to_string(),
        ),
    ])
}

pub fn items(items: &[ItemRef]) -> String {
    if items.is_empty() {
        return "No jobs found.\n".to_string();
    }
    let mut table = Table::new(&["NAME", "TYPE", "STATUS", "URL"]);
    for item in items {
        table.row(vec![
            item.name.clone(),
            item_kind(&item.class).to_string(),
            color_status(item.color.as_deref()),
            item.url.clone(),
        ]);
    }
    table.render()
}

pub fn job(job: &Job) -> String {
    let build_ref = |b: &Option<crate::models::BuildRef>| match b {
        Some(b) => format!("#{}", b.number),
        None => "-".to_string(),
    };
    let name = if job.full_name.is_empty() {
        &job.name
    } else {
        &job.full_name
    };
    let mut pairs = vec![
        ("Name", name.clone()),
        ("Type", item_kind(&job.class).to_string()),
        ("URL", job.url.clone()),
        (
            "Description",
            or_dash(job.description.as_deref()).to_string(),
        ),
    ];
    let is_container = job.color.is_none() && !job.jobs.is_empty();
    if !is_container {
        pairs.extend([
            ("Status", color_status(job.color.as_deref())),
            ("Buildable", job.buildable.map_or("-".to_string(), yes_no)),
            ("In queue", job.in_queue.map_or("-".to_string(), yes_no)),
            ("Last build", build_ref(&job.last_build)),
            ("Last success", build_ref(&job.last_successful_build)),
            ("Last failure", build_ref(&job.last_failed_build)),
        ]);
    }
    if let Some(health) = job.health_report.first() {
        pairs.push((
            "Health",
            format!("{}% ({})", health.score, health.description),
        ));
    }
    let mut out = details(&pairs);
    if !job.jobs.is_empty() {
        out.push('\n');
        out.push_str(&items(&job.jobs));
    }
    out
}

pub fn builds(builds: &[BuildSummary]) -> String {
    if builds.is_empty() {
        return "No builds found.\n".to_string();
    }
    let mut table = Table::new(&["NUMBER", "RESULT", "STARTED", "DURATION", "URL"]);
    for b in builds {
        table.row(vec![
            format!("#{}", b.number),
            build_result(b.result.as_deref(), b.building),
            format_timestamp(b.timestamp),
            if b.building {
                "-".to_string()
            } else {
                format_duration(b.duration)
            },
            b.url.clone(),
        ]);
    }
    table.render()
}

pub fn build(b: &Build) -> String {
    let causes: Vec<&str> = b.causes().collect();
    let duration = if b.building {
        format!(
            "running (estimated {})",
            format_duration(b.estimated_duration)
        )
    } else {
        format_duration(b.duration)
    };
    let mut out = details(&[
        (
            "Build",
            if b.full_display_name.is_empty() {
                format!("#{}", b.number)
            } else {
                b.full_display_name.clone()
            },
        ),
        ("Result", build_result(b.result.as_deref(), b.building)),
        ("Started", format_timestamp(b.timestamp)),
        ("Duration", duration),
        ("URL", b.url.clone()),
        ("Description", or_dash(b.description.as_deref()).to_string()),
        ("Causes", or_dash(Some(&causes.join("; "))).to_string()),
    ]);
    if !b.artifacts.is_empty() {
        out.push_str("Artifacts:\n");
        for a in &b.artifacts {
            let _ = writeln!(out, "  {}", a.relative_path);
        }
    }
    out
}

pub fn queue(queue: &Queue) -> String {
    if queue.items.is_empty() {
        return "The queue is empty.\n".to_string();
    }
    let mut table = Table::new(&["ID", "JOB", "STATE", "SINCE", "WHY"]);
    for item in &queue.items {
        table.row(vec![
            item.id.to_string(),
            item.task.name.clone(),
            queue_state(item).to_string(),
            format_timestamp(item.in_queue_since),
            or_dash(item.why.as_deref()).to_string(),
        ]);
    }
    table.render()
}

fn queue_state(item: &QueueItem) -> &'static str {
    if item.cancelled == Some(true) {
        "cancelled"
    } else if item.executable.is_some() || item.class.ends_with("LeftItem") {
        "left"
    } else if item.stuck {
        "stuck"
    } else if item.blocked {
        "blocked"
    } else if item.buildable {
        "buildable"
    } else {
        "waiting"
    }
}

pub fn queue_item(item: &QueueItem) -> String {
    let started = match &item.executable {
        Some(b) => format!("#{} {}", b.number, b.url),
        None => "-".to_string(),
    };
    details(&[
        ("ID", item.id.to_string()),
        ("Job", item.task.name.clone()),
        ("Job URL", or_dash(Some(&item.task.url)).to_string()),
        ("State", queue_state(item).to_string()),
        ("Queued", format_timestamp(item.in_queue_since)),
        ("Why", or_dash(item.why.as_deref()).to_string()),
        ("Build", started),
    ])
}

fn labels(c: &Computer) -> String {
    let names: Vec<&str> = c.assigned_labels.iter().map(|l| l.name.as_str()).collect();
    or_dash(Some(&names.join(","))).to_string()
}

pub fn computers(set: &ComputerSet) -> String {
    let mut table = Table::new(&["NAME", "STATUS", "EXECUTORS", "LABELS", "OFFLINE REASON"]);
    for c in &set.computer {
        table.row(vec![
            c.display_name.clone(),
            c.status().to_string(),
            c.num_executors.to_string(),
            labels(c),
            if c.offline {
                or_dash(c.offline_cause_reason.as_deref()).to_string()
            } else {
                "-".to_string()
            },
        ]);
    }
    let mut out = table.render();
    let _ = writeln!(
        out,
        "\n{} of {} executors busy",
        set.busy_executors, set.total_executors
    );
    out
}

pub fn computer(c: &Computer) -> String {
    let mut pairs = vec![
        ("Name", c.display_name.clone()),
        ("Description", or_dash(c.description.as_deref()).to_string()),
        ("Status", c.status().to_string()),
        ("Executors", c.num_executors.to_string()),
        ("Labels", labels(c)),
    ];
    if c.offline {
        pairs.push((
            "Offline reason",
            or_dash(c.offline_cause_reason.as_deref()).to_string(),
        ));
        pairs.push(("Taken offline", yes_no(c.temporarily_offline)));
    }
    details(&pairs)
}

pub fn views(views: &[ItemRef]) -> String {
    if views.is_empty() {
        return "No views found.\n".to_string();
    }
    let mut table = Table::new(&["NAME", "URL"]);
    for v in views {
        table.row(vec![v.name.clone(), v.url.clone()]);
    }
    table.render()
}

pub fn view(v: &View) -> String {
    let mut out = details(&[
        ("Name", v.name.clone()),
        ("Description", or_dash(v.description.as_deref()).to_string()),
        ("URL", v.url.clone()),
    ]);
    out.push('\n');
    out.push_str(&items(&v.jobs));
    out
}

pub fn plugins(pm: &PluginManager, only_updates: bool) -> String {
    let mut plugins: Vec<_> = pm
        .plugins
        .iter()
        .filter(|p| !only_updates || p.has_update)
        .collect();
    if plugins.is_empty() {
        return if only_updates {
            "All plugins are up to date.\n".to_string()
        } else {
            "No plugins installed.\n".to_string()
        };
    }
    plugins.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    let mut table = Table::new(&["NAME", "VERSION", "ENABLED", "ACTIVE", "UPDATE", "TITLE"]);
    for p in plugins {
        table.row(vec![
            p.short_name.clone(),
            p.version.clone(),
            yes_no(p.enabled),
            yes_no(p.active),
            if p.has_update { "available" } else { "-" }.to_string(),
            p.long_name.clone(),
        ]);
    }
    table.render()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_aligns_columns() {
        let mut t = Table::new(&["A", "LONG HEADER", "C"]);
        t.row(vec!["wide value".into(), "x".into(), "end".into()]);
        assert_eq!(
            t.render(),
            "A           LONG HEADER  C\nwide value  x            end\n"
        );
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(400), "<1s");
        assert_eq!(format_duration(12_000), "12s");
        assert_eq!(format_duration(83_000), "1m 23s");
        assert_eq!(format_duration(3_720_000), "1h 02m");
        assert_eq!(format_duration(-5), "0s");
    }

    #[test]
    fn timestamps() {
        assert_eq!(format_timestamp(1_761_400_000_000), "2025-10-25T13:46:40Z");
        assert_eq!(format_timestamp(0), "-");
        assert_eq!(format_timestamp(-1), "-");
    }

    #[test]
    fn building_build_shows_building() {
        let b = BuildSummary {
            number: 7,
            building: true,
            ..Default::default()
        };
        let out = builds(&[b]);
        assert!(out.contains("#7"));
        assert!(out.contains("BUILDING"));
    }

    #[test]
    fn plugin_update_filter() {
        let pm: PluginManager = serde_json::from_str(
            r#"{"plugins":[{"shortName":"git","version":"1","hasUpdate":true},{"shortName":"ant","version":"2"}]}"#,
        )
        .unwrap();
        let out = plugins(&pm, true);
        assert!(out.contains("git"));
        assert!(!out.contains("ant"));
        let all = plugins(&pm, false);
        assert!(
            all.find("ant").unwrap() < all.find("git").unwrap(),
            "sorted by name"
        );
    }
}
