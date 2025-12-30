//! Handlers for home page and newsgroup browsing.
//!
//! Displays a hierarchical group tree with statistics.
//! Prefetches group stats in the background for uncached groups.

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    response::Html,
    Extension,
};
use tracing::instrument;

use super::insert_auth_context;
use crate::error::{AppError, AppErrorResponse, ResultExt};
use crate::middleware::{CurrentUser, RequestId};
use crate::nntp::GroupTreeNode;
use crate::state::AppState;

/// Extract all group names from a list of tree nodes (recursively including children)
fn extract_all_group_names(nodes: &[GroupTreeNode]) -> Vec<String> {
    let mut names = Vec::new();
    for node in nodes {
        if let Some(ref name) = node.full_name {
            names.push(name.clone());
        }
        names.extend(extract_all_group_names(&node.children));
    }
    names
}

/// Extract group names from top-level nodes only (no recursion into children)
fn extract_top_level_group_names(nodes: &[GroupTreeNode]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| node.full_name.clone())
        .collect()
}

/// Get cached stats for groups and identify which need prefetching.
/// Returns: (cached group stats, thread counts, groups needing prefetch)
async fn get_stats_for_groups(
    state: &AppState,
    group_names: &[String],
) -> (
    HashMap<String, Option<String>>,
    HashMap<String, usize>,
    Vec<String>,
) {
    // Fetch group stats and thread counts in parallel
    let (stats_result, thread_counts) = tokio::join!(
        state.nntp.get_all_cached_group_stats(group_names),
        state.nntp.get_all_cached_thread_counts_for(group_names)
    );

    let (group_stats, needs_prefetch) = stats_result;
    (group_stats, thread_counts, needs_prefetch)
}

/// Home page handler showing all newsgroups in a tree hierarchy.
/// Only fetches stats for top-level groups, similar to /browse/{prefix}.
#[instrument(name = "home::index", skip(state, request_id, current_user))]
pub async fn index(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(current_user): Extension<CurrentUser>,
) -> Result<Html<String>, AppErrorResponse> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await.with_request_id(&request_id)?;

    // Build tree hierarchy
    let tree = GroupTreeNode::build_tree(&groups);

    // Only get stats for top-level groups (visible at root level)
    // This matches the behavior of /browse/{prefix} which only stats visible nodes
    let top_level_group_names = extract_top_level_group_names(&tree);

    // Get cached stats + identify what needs prefetching
    let (group_stats, thread_counts, needs_prefetch) =
        get_stats_for_groups(&state, &top_level_group_names).await;

    // Trigger background prefetch for uncached groups
    if !needs_prefetch.is_empty() {
        state.nntp.prefetch_group_stats(needs_prefetch);
    }

    // Build tree with available stats
    let tree_with_stats =
        GroupTreeNode::build_tree_with_stats(&groups, &thread_counts, &group_stats);

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("groups", &groups);
    context.insert("nodes", &tree_with_stats);
    context.insert("path", "");
    context.insert("breadcrumbs", &Vec::<(&str, &str)>::new());
    context.insert("group_stats", &group_stats);
    context.insert("thread_counts", &thread_counts);

    insert_auth_context(&mut context, &state, &current_user, false);

    let html = state
        .tera
        .render("home.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;
    Ok(Html(html))
}

/// Browse handler for navigating into group hierarchy by prefix path.
#[instrument(name = "home::browse", skip(state, request_id, current_user), fields(prefix = %prefix))]
pub async fn browse(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(current_user): Extension<CurrentUser>,
    Path(prefix): Path<String>,
) -> Result<Html<String>, AppErrorResponse> {
    // Fetch all groups (cached + coalesced)
    let groups = state.nntp.get_groups().await.with_request_id(&request_id)?;

    // Build initial tree to find which groups are visible at this path
    let initial_tree = GroupTreeNode::build_tree(&groups);
    let visible_nodes =
        GroupTreeNode::find_children_at_path(&initial_tree, &prefix).unwrap_or_default();

    // Also check if the current path itself is a group
    let current_node = GroupTreeNode::find_node_at_path(&initial_tree, &prefix);

    // Collect all group names from visible nodes + current node
    let mut all_group_names = extract_all_group_names(&visible_nodes);
    if let Some(ref node) = current_node {
        if let Some(ref name) = node.full_name {
            if !all_group_names.contains(name) {
                all_group_names.push(name.clone());
            }
        }
    }

    // Get cached stats + identify what needs prefetching
    let (group_stats, thread_counts, needs_prefetch) =
        get_stats_for_groups(&state, &all_group_names).await;

    // Trigger background prefetch for uncached groups
    if !needs_prefetch.is_empty() {
        state.nntp.prefetch_group_stats(needs_prefetch);
    }

    // Build tree hierarchy with stats
    let tree = GroupTreeNode::build_tree_with_stats(&groups, &thread_counts, &group_stats);

    // Find children at the given path
    let nodes_with_stats = GroupTreeNode::find_children_at_path(&tree, &prefix)
        .ok_or_else(|| AppError::Internal(format!("Path not found: {}", prefix)))
        .with_request_id(&request_id)?;

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
    context.insert("nodes", &nodes_with_stats);
    context.insert("path", &prefix);
    context.insert("breadcrumbs", &breadcrumbs);
    context.insert("current_node", &current_node);
    context.insert("group_stats", &group_stats);
    context.insert("thread_counts", &thread_counts);

    insert_auth_context(&mut context, &state, &current_user, false);

    let html = state
        .tera
        .render("home.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;
    Ok(Html(html))
}
