use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    response::Html,
};

use crate::error::AppError;
use crate::nntp::GroupTreeNode;
use crate::state::AppState;

/// Extract all group names from a list of tree nodes (including the node itself if it's a group)
fn extract_group_names(nodes: &[GroupTreeNode]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|n| n.full_name.clone())
        .collect()
}

/// Fetch group stats for a list of group names concurrently
async fn fetch_stats_for_groups(
    state: &AppState,
    group_names: &[String],
) -> (HashMap<String, Option<String>>, HashMap<String, usize>) {
    use futures::future::join_all;

    // Fetch group stats (last_post_date) for visible groups concurrently
    let stats_futures: Vec<_> = group_names
        .iter()
        .map(|name| {
            let state = state.clone();
            let name = name.clone();
            async move {
                let result = state.nntp.get_group_stats(&name).await.ok();
                (name, result.and_then(|s| s.last_article_date))
            }
        })
        .collect();

    let stats_results = join_all(stats_futures).await;
    let group_stats: HashMap<String, Option<String>> = stats_results.into_iter().collect();

    // Get cached thread counts (from threads cache - only populated after visiting a group)
    let thread_counts = state.nntp.get_all_cached_thread_counts_for(group_names).await;

    (group_stats, thread_counts)
}

pub async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await?;

    // Build initial tree to find which groups are visible at the root level
    let initial_tree = GroupTreeNode::build_tree(&groups);
    let visible_group_names = extract_group_names(&initial_tree);

    // Fetch stats only for visible groups
    let (group_stats, thread_counts) = fetch_stats_for_groups(&state, &visible_group_names).await;

    // Build tree hierarchy from flat group list with stats
    let tree = GroupTreeNode::build_tree_with_stats(&groups, &thread_counts, &group_stats);

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("groups", &groups);
    context.insert("nodes", &tree);
    context.insert("path", "");
    context.insert("breadcrumbs", &Vec::<(&str, &str)>::new());
    context.insert("group_stats", &group_stats);
    context.insert("thread_counts", &thread_counts);

    let html = state.tera.render("home.html", &context)?;
    Ok(Html(html))
}

pub async fn browse(
    State(state): State<AppState>,
    Path(prefix): Path<String>,
) -> Result<Html<String>, AppError> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await?;

    // Build initial tree to find which groups are visible at this path
    let initial_tree = GroupTreeNode::build_tree(&groups);
    let visible_nodes = GroupTreeNode::find_children_at_path(&initial_tree, &prefix)
        .unwrap_or_default();
    
    // Also check if the current path itself is a group
    let current_node = GroupTreeNode::find_node_at_path(&initial_tree, &prefix);
    
    // Collect all visible group names
    let mut visible_group_names = extract_group_names(&visible_nodes);
    if let Some(ref node) = current_node {
        if let Some(ref name) = node.full_name {
            if !visible_group_names.contains(name) {
                visible_group_names.push(name.clone());
            }
        }
    }

    // Fetch stats only for visible groups
    let (group_stats, thread_counts) = fetch_stats_for_groups(&state, &visible_group_names).await;

    // Build tree hierarchy with stats
    let tree = GroupTreeNode::build_tree_with_stats(&groups, &thread_counts, &group_stats);

    // Find children at the given path
    let nodes = GroupTreeNode::find_children_at_path(&tree, &prefix)
        .ok_or_else(|| AppError::Internal(format!("Path not found: {}", prefix)))?;

    // Find the current node (to check if it's also a group)
    let current_node = GroupTreeNode::find_node_at_path(&tree, &prefix);

    // Build breadcrumbs
    let parts: Vec<&str> = prefix.split('.').collect();
    let mut breadcrumbs: Vec<(String, String)> = Vec::new();
    let mut accumulated = String::new();
    for part in &parts {
        if !accumulated.is_empty() {
            accumulated.push('.');
        }
        accumulated.push_str(part);
        breadcrumbs.push((part.to_string(), accumulated.clone()));
    }

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("groups", &groups);
    context.insert("nodes", &nodes);
    context.insert("path", &prefix);
    context.insert("breadcrumbs", &breadcrumbs);
    context.insert("current_node", &current_node);
    context.insert("group_stats", &group_stats);
    context.insert("thread_counts", &thread_counts);

    let html = state.tera.render("home.html", &context)?;
    Ok(Html(html))
}
