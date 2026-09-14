//! Turning the rows an EXPLAIN returns into a tree.
//!
//! Three servers print three different things and none of them is a data
//! structure: Postgres a block of indented text in one `QUERY PLAN` column,
//! MySQL the same idea crammed into a single cell with embedded newlines, and
//! SQLite a four-column table whose shape lives in `id`/`parent` rather than in
//! whitespace. There is no structured format they share — `FORMAT JSON` is
//! Postgres-only, and asking for it would mean rewriting the statement the user
//! asked to run, which rule 1 forbids. So we parse what arrived.
//!
//! Which means this parses text from servers across versions nobody here has
//! seen. Nothing in it may panic and nothing may be dropped: a line that makes
//! no sense becomes a node carrying its own raw text, and a metric that does
//! not parse stays in the label where the user can still read it.

/// A parsed EXPLAIN, ready to render.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    /// Flattened depth-first, which is the order the server printed them and
    /// the order they render in.
    pub nodes: Vec<PlanNode>,
    /// The trailing `Planning Time: … ms` / `Execution Time: … ms` style lines,
    /// as label and value.
    pub summary: Vec<(String, String)>,
    /// What the bars are a share of. `None` when the plan carried no timings.
    pub total_ms: Option<f64>,
    /// The server's plan exactly as it arrived.
    pub text: String,
}

/// One operator in the plan.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanNode {
    pub depth: usize,
    /// The operator and its target: `Seq Scan on accounts`.
    pub label: String,
    /// The qualifier lines printed under the node — `Filter: …`,
    /// `Index Cond: …`, `Buffers: …` — verbatim and in order.
    pub detail: Vec<String>,
    /// `cost=0.00..35.50 rows=2550 width=4`, when the plan carried estimates.
    pub estimated: Option<Estimated>,
    /// `actual time=0.01..0.02 rows=10 loops=1`, when it was an ANALYZE.
    pub actual: Option<Actual>,
    /// Time in this node alone — its own total less its children's — in
    /// milliseconds, and what the bar is drawn from. `None` without timings.
    pub self_ms: Option<f64>,
}

/// What the planner guessed. MySQL prints one cost and no width, so
/// `startup_cost` and `total_cost` are the same number and `width` is 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimated {
    pub startup_cost: f64,
    pub total_cost: f64,
    pub rows: u64,
    pub width: u64,
}

/// What actually happened. `rows` and `loops` are floats because Postgres
/// prints an average over the loops, which is fractional.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Actual {
    pub startup_ms: f64,
    pub total_ms: f64,
    pub rows: f64,
    pub loops: f64,
}

impl Actual {
    /// The time the node took across every loop, which is what a parent's own
    /// total already contains and so what a parent subtracts.
    fn inclusive_ms(&self) -> f64 {
        self.total_ms * self.loops
    }
}

/// Parse the rows an EXPLAIN returned into a plan.
pub fn parse(columns: &[String], rows: &[Vec<Option<String>>]) -> Plan {
    if let Some(plan) = parse_linked(columns, rows) {
        return plan;
    }

    let text = rows
        .iter()
        .flatten()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    parse_indented(text)
}

/// SQLite's `EXPLAIN QUERY PLAN`, which is a table and not a drawing: the tree
/// is in the `id`/`parent` linkage, and indentation would be a guess.
fn parse_linked(columns: &[String], rows: &[Vec<Option<String>>]) -> Option<Plan> {
    if columns.len() != 4 {
        return None;
    }
    let column = |name: &str| columns.iter().position(|c| c.eq_ignore_ascii_case(name));
    let (id, parent, detail) = (column("id")?, column("parent")?, column("detail")?);
    column("notused")?;

    let mut depths: Vec<(i64, usize)> = Vec::new();
    let mut nodes = Vec::new();
    for row in rows {
        let cell = |index: usize| {
            row.get(index)
                .and_then(Option::as_deref)
                .unwrap_or_default()
                .trim()
        };
        // A parent of 0 is a root, and so is a parent we have not seen — a
        // forward reference would otherwise have no depth to hang from.
        let depth = cell(parent)
            .parse::<i64>()
            .ok()
            .and_then(|parent| depths.iter().find(|(id, _)| *id == parent))
            .map_or(0, |(_, depth)| depth + 1);
        if let Ok(id) = cell(id).parse::<i64>() {
            depths.push((id, depth));
        }
        nodes.push(PlanNode {
            depth,
            label: cell(detail).to_string(),
            ..PlanNode::default()
        });
    }

    let text = nodes
        .iter()
        .map(|node| node.label.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(Plan {
        nodes,
        text,
        ..Plan::default()
    })
}

/// Postgres and MySQL, where depth is drawn rather than stated.
fn parse_indented(text: String) -> Plan {
    let mut nodes: Vec<PlanNode> = Vec::new();
    let mut summary: Vec<(String, String)> = Vec::new();
    // The indent column of every node on the path down to the last one. Depth
    // is that path's length, which holds for Postgres' 2-then-4 steps, MySQL's
    // 4 and SQLite's 3 without any of them being written down here.
    let mut ancestors: Vec<usize> = Vec::new();

    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let (indent, body, opens_node) = strip_marker(line);

        if opens_node || nodes.is_empty() {
            while ancestors.last().is_some_and(|last| *last >= indent) {
                ancestors.pop();
            }
            let (label, estimated, actual) = split_metrics(body);
            nodes.push(PlanNode {
                depth: ancestors.len(),
                label,
                detail: Vec::new(),
                estimated,
                actual,
                self_ms: None,
            });
            ancestors.push(indent);
        } else if indent == 0
            && let Some((label, value)) = body.split_once(':')
        {
            summary.push((label.trim().to_string(), value.trim().to_string()));
        } else if let Some(node) = nodes.last_mut() {
            node.detail.push(body.to_string());
        }
    }

    fill_self_ms(&mut nodes);
    let total_ms = summary
        .iter()
        .find(|(label, _)| label.eq_ignore_ascii_case("Execution Time"))
        .and_then(|(_, value)| value.split_whitespace().next()?.parse().ok())
        .or_else(|| Some(nodes.first()?.actual?.inclusive_ms()));

    Plan {
        nodes,
        summary,
        total_ms,
        text,
    }
}

/// The indent column, the line without its tree drawing, and whether that
/// drawing opened a node. Postgres and MySQL draw with `->`, SQLite's indented
/// form with `|--` and `` `-- `` under runs of `|  `.
fn strip_marker(line: &str) -> (usize, &str, bool) {
    let indent = line
        .bytes()
        .take_while(|byte| *byte == b' ' || *byte == b'|')
        .count();
    let rest = &line[indent..];

    for marker in ["->", "`--", "--"] {
        if let Some(body) = rest.strip_prefix(marker) {
            return (indent, body.trim_start(), true);
        }
    }
    (indent, rest, false)
}

/// Cut the `(cost=…)` and `(actual …)` parentheticals off a node's line; what
/// is left is the label. One that does not parse is left where it was, because
/// a number we cannot read is still a number the user can.
fn split_metrics(line: &str) -> (String, Option<Estimated>, Option<Actual>) {
    let mut label = line.to_string();
    let estimated = cut(&mut label, "(cost=", parse_estimated);
    let actual = cut(&mut label, "(actual ", parse_actual);
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    (label, estimated, actual)
}

fn cut<T>(label: &mut String, open: &str, parse: fn(&str) -> Option<T>) -> Option<T> {
    let start = label.find(open)?;
    let close = start + label[start..].find(')')?;
    let parsed = parse(&label[start + open.len()..close])?;
    label.replace_range(start..=close, "");
    Some(parsed)
}

/// `0.00..35.50 rows=2550 width=8`, and MySQL's `1.25 rows=10`.
fn parse_estimated(inner: &str) -> Option<Estimated> {
    let mut fields = inner.split_whitespace();
    let (startup_cost, total_cost) = pair(fields.next()?)?;
    let (mut rows, mut width) = (0, 0);
    for field in fields {
        if let Some(value) = field.strip_prefix("rows=") {
            rows = value.parse().ok()?;
        } else if let Some(value) = field.strip_prefix("width=") {
            width = value.parse().ok()?;
        }
    }
    Some(Estimated {
        startup_cost,
        total_cost,
        rows,
        width,
    })
}

/// `time=0.015..0.019 rows=10 loops=1`. Without a `time=` there is nothing to
/// draw a bar from — `(never executed)` and `TIMING OFF` both land here — so
/// the parenthetical stays in the label instead.
fn parse_actual(inner: &str) -> Option<Actual> {
    let mut time = None;
    let (mut rows, mut loops) = (0.0, 1.0);
    for field in inner.split_whitespace() {
        if let Some(value) = field.strip_prefix("time=") {
            time = Some(pair(value)?);
        } else if let Some(value) = field.strip_prefix("rows=") {
            rows = value.parse().ok()?;
        } else if let Some(value) = field.strip_prefix("loops=") {
            loops = value.parse().ok()?;
        }
    }
    let (startup_ms, total_ms) = time?;
    Some(Actual {
        startup_ms,
        total_ms,
        rows,
        loops,
    })
}

/// `A..B`, or MySQL's bare `A` where startup and total are the same number.
fn pair(value: &str) -> Option<(f64, f64)> {
    match value.split_once("..") {
        Some((start, end)) => Some((start.parse().ok()?, end.parse().ok()?)),
        None => {
            let single = value.parse().ok()?;
            Some((single, single))
        }
    }
}

fn fill_self_ms(nodes: &mut [PlanNode]) {
    let inclusive: Vec<Option<f64>> = nodes
        .iter()
        .map(|node| node.actual.map(|actual| actual.inclusive_ms()))
        .collect();

    // ponytail: rescanning forward for each node is quadratic; plans are tens
    // of lines, and a child-index pass is the upgrade if one ever is not.
    for index in 0..nodes.len() {
        let Some(own) = inclusive[index] else {
            continue;
        };
        let depth = nodes[index].depth;
        let mut children = 0.0;
        for (sibling, node) in nodes.iter().enumerate().skip(index + 1) {
            if node.depth <= depth {
                break;
            }
            if node.depth == depth + 1 {
                children += inclusive[sibling].unwrap_or_default();
            }
        }
        // Estimates and loop averaging make small negatives normal, and a
        // negative bar is nonsense.
        nodes[index].self_ms = Some((own - children).max(0.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_column(text: &str) -> Plan {
        parse(&["QUERY PLAN".to_string()], &[vec![Some(text.to_string())]])
    }

    /// Timings never subtract cleanly in binary, so assert to a tolerance far
    /// tighter than any difference the parser could introduce.
    #[track_caller]
    fn close(actual: Option<f64>, expected: f64) {
        let Some(actual) = actual else {
            panic!("expected {expected}, got None");
        };
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn postgres_nests_by_indentation_and_keeps_its_qualifiers() {
        let plan = one_column(
            r#"Limit  (cost=0.00..0.29 rows=10 width=8) (actual time=0.015..0.019 rows=10 loops=1)
  ->  Nested Loop  (cost=0.00..1.00 rows=10 width=8) (actual time=0.014..0.018 rows=10 loops=1)
        ->  Seq Scan on accounts  (cost=0.00..35.50 rows=2550 width=8) (actual time=0.013..0.015 rows=10 loops=1)
              Filter: (id > 5)
              Rows Removed by Filter: 5
        ->  Index Scan using orders_pkey on orders  (cost=0.29..8.31 rows=1 width=4) (actual time=0.001..0.002 rows=1 loops=1)
Planning Time: 0.083 ms
Execution Time: 0.041 ms"#,
        );

        let shape: Vec<(usize, &str)> = plan
            .nodes
            .iter()
            .map(|node| (node.depth, node.label.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, "Limit"),
                (1, "Nested Loop"),
                (2, "Seq Scan on accounts"),
                (2, "Index Scan using orders_pkey on orders"),
            ]
        );

        // A qualifier belongs to the node above it however far it is indented
        // past it, and comes back verbatim.
        assert_eq!(
            plan.nodes[2].detail,
            vec!["Filter: (id > 5)", "Rows Removed by Filter: 5"]
        );
        assert!(plan.nodes[3].detail.is_empty());

        assert_eq!(
            plan.nodes[2].estimated,
            Some(Estimated {
                startup_cost: 0.00,
                total_cost: 35.50,
                rows: 2550,
                width: 8,
            })
        );
        assert_eq!(
            plan.nodes[0].actual,
            Some(Actual {
                startup_ms: 0.015,
                total_ms: 0.019,
                rows: 10.0,
                loops: 1.0,
            })
        );

        assert_eq!(
            plan.summary,
            vec![
                ("Planning Time".to_string(), "0.083 ms".to_string()),
                ("Execution Time".to_string(), "0.041 ms".to_string()),
            ]
        );
        // The reported execution time wins over the root's own, because it is
        // the number the user is shown.
        close(plan.total_ms, 0.041);
        assert!(plan.text.starts_with("Limit  (cost="));
    }

    #[test]
    fn a_nodes_own_time_excludes_the_time_of_its_children() {
        let plan = one_column(
            r#"Nested Loop  (cost=0.00..2.00 rows=10 width=8) (actual time=0.000..1.000 rows=10 loops=1)
  ->  Seq Scan on a  (cost=0.00..1.00 rows=10 width=8) (actual time=0.000..0.400 rows=10 loops=1)
  ->  Index Scan using b_pkey on b  (cost=0.00..1.00 rows=1 width=8) (actual time=0.000..0.020 rows=1 loops=10)
        Index Cond: (b.id = a.id)"#,
        );

        // The inner scan ran ten times, so it cost its parent 0.2 and not 0.02.
        close(plan.nodes[2].self_ms, 0.2);
        close(plan.nodes[1].self_ms, 0.4);
        close(plan.nodes[0].self_ms, 1.0 - 0.4 - 0.2);
        // Without an Execution Time line the root's inclusive time is the whole.
        close(plan.total_ms, 1.0);

        let impossible = one_column(
            r#"Nested Loop  (actual time=0.000..1.000 rows=1 loops=1)
  ->  Seq Scan on a  (actual time=0.000..2.000 rows=1 loops=1)"#,
        );
        assert_eq!(impossible.nodes[0].self_ms, Some(0.0));
        assert_eq!(impossible.nodes[0].estimated, None);
    }

    #[test]
    fn mysql_arrives_as_one_cell_of_newlines_and_a_single_cost() {
        let plan = parse(
            &["EXPLAIN".to_string()],
            &[vec![Some(
                "-> Limit: 10 row(s)  (cost=1.25 rows=10) (actual time=0.021..0.030 rows=10 loops=1)\n    -> Table scan on accounts  (cost=2.50 rows=12) (actual time=0.019..0.025 rows=12 loops=1)\n"
                    .to_string(),
            )]],
        );

        assert_eq!(plan.nodes.len(), 2);
        assert_eq!(plan.nodes[0].depth, 0);
        assert_eq!(plan.nodes[0].label, "Limit: 10 row(s)");
        assert_eq!(plan.nodes[1].depth, 1);
        assert_eq!(plan.nodes[1].label, "Table scan on accounts");

        // One cost, so startup and total are it, and there is no width.
        assert_eq!(
            plan.nodes[1].estimated,
            Some(Estimated {
                startup_cost: 2.50,
                total_cost: 2.50,
                rows: 12,
                width: 0,
            })
        );
        close(plan.total_ms, 0.030);
        close(plan.nodes[0].self_ms, 0.030 - 0.025);
    }

    #[test]
    fn sqlite_takes_its_depth_from_the_parent_column_not_the_page() {
        let columns = ["id", "parent", "notused", "detail"].map(str::to_string);
        let row = |id: &str, parent: &str, detail: &str| {
            vec![
                Some(id.to_string()),
                Some(parent.to_string()),
                Some("0".to_string()),
                Some(detail.to_string()),
            ]
        };
        let plan = parse(
            &columns,
            &[
                row("2", "0", "SCAN accounts"),
                row("6", "2", "SEARCH orders USING INDEX orders_account"),
                row("4", "0", "USE TEMP B-TREE FOR ORDER BY"),
            ],
        );

        let shape: Vec<(usize, &str)> = plan
            .nodes
            .iter()
            .map(|node| (node.depth, node.label.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, "SCAN accounts"),
                (1, "SEARCH orders USING INDEX orders_account"),
                (0, "USE TEMP B-TREE FOR ORDER BY"),
            ]
        );
        assert_eq!(plan.total_ms, None);
        assert!(plan.summary.is_empty());
        assert!(plan.nodes.iter().all(|node| node.actual.is_none()));
        assert!(plan.nodes.iter().all(|node| node.self_ms.is_none()));

        // Some builds hand back the drawn tree in one column instead, where
        // depth is back to being indentation.
        let drawn = one_column(
            "|--SCAN accounts\n|  `--SEARCH orders USING INDEX\n`--USE TEMP B-TREE FOR ORDER BY",
        );
        let shape: Vec<(usize, &str)> = drawn
            .nodes
            .iter()
            .map(|node| (node.depth, node.label.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, "SCAN accounts"),
                (1, "SEARCH orders USING INDEX"),
                (0, "USE TEMP B-TREE FOR ORDER BY"),
            ]
        );
    }

    #[test]
    fn a_line_nobody_can_read_still_renders_as_itself() {
        let plan = one_column(
            r#"Limit  (cost=oops rows=x width=) (actual whatever)
  ->  ???  (cost=0.00..1.00 rows=1 width=1)
  not a node at all
Planning Time: broken"#,
        );

        // Nothing is dropped: metrics that did not parse stay readable in the
        // label they came from.
        assert_eq!(
            plan.nodes[0].label,
            "Limit (cost=oops rows=x width=) (actual whatever)"
        );
        assert_eq!(plan.nodes[0].estimated, None);
        assert_eq!(plan.nodes[0].actual, None);
        assert_eq!(plan.nodes[0].self_ms, None);

        assert_eq!(plan.nodes[1].label, "???");
        assert_eq!(plan.nodes[1].detail, vec!["not a node at all"]);
        assert_eq!(
            plan.summary,
            vec![("Planning Time".to_string(), "broken".to_string())]
        );
        assert_eq!(plan.total_ms, None);

        assert_eq!(parse(&[], &[]), Plan::default());
        assert_eq!(one_column("").nodes, Vec::new());
    }
}
