//! NNTP client module providing Usenet/newsgroup access.
//!
//! This module contains data types for representing articles, threads, and newsgroups,
//! as well as thread-building logic that constructs threaded views from NNTP OVER and HDR
//! command responses.
//!
//! Key re-exports:
//! - [`NntpFederatedService`] - Federated NNTP service for multi-server access

mod federated;
mod messages;
mod service;
mod tls;
mod worker;

pub use federated::NntpFederatedService;

use std::collections::HashMap;

use nntp_rs::OverviewEntry;
use serde::Serialize;

use crate::config::{DEFAULT_SUBJECT, PAGINATION_WINDOW};

/// Pagination state for paginated list views.
#[derive(Debug, Clone, Serialize)]
pub struct PaginationInfo {
    pub current_page: usize,
    pub total_pages: usize,
    pub total_items: usize,
    pub items_per_page: usize,
    pub has_prev: bool,
    pub has_next: bool,
    /// Visible page numbers for navigation (e.g., [1, 2, 3, 4, 5])
    pub visible_pages: Vec<usize>,
}

impl PaginationInfo {
    pub fn new(current_page: usize, total_items: usize, items_per_page: usize) -> Self {
        let total_pages = if total_items == 0 {
            1
        } else {
            total_items.div_ceil(items_per_page)
        };

        let visible_pages = Self::compute_visible_pages(current_page, total_pages);

        Self {
            current_page,
            total_pages,
            total_items,
            items_per_page,
            has_prev: current_page > 1,
            has_next: current_page < total_pages,
            visible_pages,
        }
    }

    fn compute_visible_pages(current: usize, total: usize) -> Vec<usize> {
        let start = current.saturating_sub(PAGINATION_WINDOW).max(1);
        let end = (current + PAGINATION_WINDOW).min(total);
        (start..=end).collect()
    }
}

/// Thread metadata including root message-id, subject, dates, and reply count.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadView {
    pub subject: String,
    pub root_message_id: String,
    pub article_count: usize,
    pub root: ThreadNodeView,
    /// Date of the most recent post in the thread
    pub last_post_date: Option<String>,
}

/// Node in a threaded article tree with child replies.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadNodeView {
    pub message_id: String,
    pub article: Option<ArticleView>,
    pub replies: Vec<ThreadNodeView>,
    /// Pre-computed count of all descendants (cached during tree construction)
    #[serde(skip)]
    pub descendant_count: usize,
}

/// Flattened article for paginated display with nesting depth info.
#[derive(Debug, Clone, Serialize)]
pub struct FlatComment {
    pub message_id: String,
    pub article: Option<ArticleView>,
    pub depth: usize,
    /// Number of descendant replies (for collapse UI)
    pub descendant_count: usize,
    /// Whether this comment starts a collapsed section
    pub starts_collapsed: bool,
}

impl ThreadNodeView {
    /// Check if a message_id exists anywhere in this node or its descendants.
    /// Uses iteration instead of recursion to avoid stack overflow.
    pub fn contains_message_id(&self, target_id: &str) -> bool {
        let mut stack: Vec<&ThreadNodeView> = vec![self];

        while let Some(node) = stack.pop() {
            if node.message_id == target_id {
                return true;
            }
            for reply in &node.replies {
                stack.push(reply);
            }
        }

        false
    }

    /// Flatten the thread tree into a list for non-recursive rendering.
    /// Uses iteration instead of recursion to avoid stack overflow.
    pub fn flatten(&self, collapse_threshold: usize) -> Vec<FlatComment> {
        let mut result = Vec::new();
        // Stack of (node, depth)
        let mut stack: Vec<(&ThreadNodeView, usize)> = vec![(self, 0)];

        while let Some((node, depth)) = stack.pop() {
            // Use pre-computed descendant count instead of walking the tree
            let starts_collapsed = depth >= collapse_threshold && !node.replies.is_empty();

            result.push(FlatComment {
                message_id: node.message_id.clone(),
                article: node.article.clone(),
                depth,
                descendant_count: node.descendant_count,
                starts_collapsed,
            });

            // Add replies in reverse order so they're processed in correct order
            for reply in node.replies.iter().rev() {
                stack.push((reply, depth + 1));
            }
        }

        result
    }

    /// Flatten and return pagination info with message IDs for the current page.
    /// Returns (all_flattened, pagination_info, message_ids_for_page)
    pub fn flatten_paginated(
        &self,
        page: usize,
        per_page: usize,
        collapse_threshold: usize,
    ) -> (Vec<FlatComment>, PaginationInfo, Vec<String>) {
        let all_flat = self.flatten(collapse_threshold);
        let total = all_flat.len();
        let pagination = PaginationInfo::new(page, total, per_page);

        // Determine which message IDs are on the current page
        let start = (page - 1) * per_page;
        let end = (start + per_page).min(total);

        let message_ids: Vec<String> = if start < total {
            all_flat[start..end]
                .iter()
                .map(|c| c.message_id.clone())
                .collect()
        } else {
            Vec::new()
        };

        (all_flat, pagination, message_ids)
    }
}

/// Parsed article with headers and body for display.
#[derive(Debug, Clone, Serialize)]
pub struct ArticleView {
    pub message_id: String,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub body: Option<String>,
    /// Raw headers for full header display (only populated for single article view)
    pub headers: Option<String>,
}

/// Newsgroup metadata including name, description, and article counts.
#[derive(Debug, Clone, Serialize)]
pub struct GroupView {
    pub name: String,
    pub description: Option<String>,
    pub article_count: Option<u64>,
}

/// Node in a hierarchical newsgroup tree for navigation.
#[derive(Debug, Clone, Serialize)]
pub struct GroupTreeNode {
    /// The segment name (e.g., "comp" or "lang" or "python")
    pub segment: String,
    /// Full group name if this is an actual group
    pub full_name: Option<String>,
    /// Group description (if this is an actual group)
    pub description: Option<String>,
    /// Child nodes (sorted alphabetically by segment)
    pub children: Vec<GroupTreeNode>,
    /// Number of threads in this group (populated after visiting the group)
    pub thread_count: Option<usize>,
    /// RFC 2822 date of the most recent article
    pub last_post_date: Option<String>,
}

impl GroupTreeNode {
    /// Build a tree from a list of groups
    pub fn build_tree(groups: &[GroupView]) -> Vec<GroupTreeNode> {
        let mut root_children: Vec<GroupTreeNode> = Vec::new();
        let mut root_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        // Sort groups alphabetically
        let mut sorted_groups: Vec<&GroupView> = groups.iter().collect();
        sorted_groups.sort_by(|a, b| a.name.cmp(&b.name));

        for group in sorted_groups {
            let parts: Vec<&str> = group.name.split('.').collect();
            Self::insert_path(&mut root_children, &mut root_map, &parts, &group.name, &group.description, None, None);
        }

        root_children
    }

    /// Build a tree from a list of groups with thread counts and last post dates
    /// - thread_counts: map of group name to thread count (from threads cache, populated after visiting)
    /// - group_stats: map of group name to last_post_date (from group stats, fetched eagerly)
    pub fn build_tree_with_stats(
        groups: &[GroupView],
        thread_counts: &std::collections::HashMap<String, usize>,
        group_stats: &std::collections::HashMap<String, Option<String>>,
    ) -> Vec<GroupTreeNode> {
        let mut root_children: Vec<GroupTreeNode> = Vec::new();
        let mut root_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        // Sort groups alphabetically
        let mut sorted_groups: Vec<&GroupView> = groups.iter().collect();
        sorted_groups.sort_by(|a, b| a.name.cmp(&b.name));

        for group in sorted_groups {
            let parts: Vec<&str> = group.name.split('.').collect();
            let thread_count = thread_counts.get(&group.name).copied();
            let last_post_date = group_stats.get(&group.name).and_then(|d| d.clone());
            Self::insert_path(&mut root_children, &mut root_map, &parts, &group.name, &group.description, thread_count, last_post_date);
        }

        root_children
    }

    fn insert_path(
        nodes: &mut Vec<GroupTreeNode>,
        node_map: &mut std::collections::HashMap<String, usize>,
        parts: &[&str],
        full_name: &str,
        description: &Option<String>,
        thread_count: Option<usize>,
        last_post_date: Option<String>,
    ) {
        if parts.is_empty() {
            return;
        }

        let segment = parts[0];
        let remaining = &parts[1..];

        // Use HashMap for O(1) lookup instead of O(n) Vec scan
        let node_idx = if let Some(&idx) = node_map.get(segment) {
            idx
        } else {
            let idx = nodes.len();
            nodes.push(GroupTreeNode {
                segment: segment.to_string(),
                full_name: None,
                description: None,
                children: Vec::new(),
                thread_count: None,
                last_post_date: None,
            });
            node_map.insert(segment.to_string(), idx);
            idx
        };

        let node = &mut nodes[node_idx];

        if remaining.is_empty() {
            // This is a leaf node - an actual group
            node.full_name = Some(full_name.to_string());
            node.description = description.clone();
            node.thread_count = thread_count;
            node.last_post_date = last_post_date;
        } else {
            // Continue down the tree - create a new HashMap for this level
            let mut child_map = std::collections::HashMap::new();
            // Build map from existing children
            for (i, child) in node.children.iter().enumerate() {
                child_map.insert(child.segment.clone(), i);
            }
            Self::insert_path(&mut node.children, &mut child_map, remaining, full_name, description, thread_count, last_post_date);
        }
    }

    /// Find children at a given path (e.g., "comp.lang" returns children of comp.lang)
    pub fn find_children_at_path(roots: &[GroupTreeNode], path: &str) -> Option<Vec<GroupTreeNode>> {
        if path.is_empty() {
            return Some(roots.to_vec());
        }

        let parts: Vec<&str> = path.split('.').collect();
        let mut current = roots;

        for part in &parts {
            let found = current.iter().find(|n| n.segment == *part)?;
            current = &found.children;
        }

        Some(current.to_vec())
    }

    /// Check if this node or any path prefix is an actual group
    pub fn find_node_at_path(roots: &[GroupTreeNode], path: &str) -> Option<GroupTreeNode> {
        if path.is_empty() {
            return None;
        }

        let parts: Vec<&str> = path.split('.').collect();
        let mut current = roots;

        for (i, part) in parts.iter().enumerate() {
            let found = current.iter().find(|n| n.segment == *part)?;
            if i == parts.len() - 1 {
                return Some(found.clone());
            }
            current = &found.children;
        }

        None
    }
}

/// Parse a raw NNTP article into an [`ArticleView`].
pub fn parse_article(article: &nntp_rs::Article) -> ArticleView {
    // Extract raw headers as string for display
    let headers = article
        .raw_headers()
        .map(|h| String::from_utf8_lossy(h).to_string());

    ArticleView {
        message_id: article.article_id().to_string(),
        subject: article.subject().unwrap_or_default(),
        from: article.from().unwrap_or_default(),
        date: article.date().unwrap_or_default(),
        body: article.body_text(),
        headers,
    }
}

/// Build a thread list from NNTP OVER command response data.
///
/// Uses the References header to reconstruct thread structure.
pub fn build_threads_from_overview(entries: Vec<OverviewEntry>) -> Vec<ThreadView> {
    if entries.is_empty() {
        return Vec::new();
    }

    // Build a map of message_id -> OverviewEntry for quick lookup
    let mut entries_by_id: HashMap<String, &OverviewEntry> = HashMap::new();
    for entry in &entries {
        if let Some(msg_id) = entry.message_id() {
            entries_by_id.insert(msg_id.to_string(), entry);
        }
    }

    // Group entries by thread root (first message in references chain, or self if no references)
    let mut threads_map: HashMap<String, Vec<&OverviewEntry>> = HashMap::new();

    for entry in &entries {
        let msg_id = match entry.message_id() {
            Some(id) => id.to_string(),
            None => continue,
        };

        // Parse references to find thread root
        let root_id = if let Some(refs) = entry.references() {
            if refs.trim().is_empty() {
                // No references - this is a root message
                msg_id.clone()
            } else {
                // First reference is the thread root
                refs.split_whitespace()
                    .next()
                    .unwrap_or(&msg_id)
                    .to_string()
            }
        } else {
            // No references field - this is a root message
            msg_id.clone()
        };

        threads_map.entry(root_id).or_default().push(entry);
    }

    // Build ThreadView for each thread
    let mut thread_views: Vec<ThreadView> = Vec::new();

    for (root_id, thread_entries) in threads_map {
        // Find the actual root entry (might not be in our entries if it's older/expired)
        let root_entry = thread_entries.iter()
            .find(|e| e.message_id() == Some(&root_id));

        // Get subject from root entry if available, otherwise from first available entry
        let subject = root_entry
            .or_else(|| thread_entries.first())
            .and_then(|e| e.subject())
            .unwrap_or(DEFAULT_SUBJECT)
            .to_string();

        // Build the tree structure using original root_id
        // If root article is missing, build_node_from_entry will create a node with article: None
        let root_node = build_thread_tree(&root_id, &thread_entries, &entries_by_id);
        let last_post_date = find_latest_date_overview(&thread_entries);

        thread_views.push(ThreadView {
            subject,
            // Always use original root_id so thread can be found even if root article is missing
            root_message_id: root_id,
            article_count: thread_entries.len(),
            root: root_node,
            last_post_date,
        });
    }

    thread_views
}

/// Build a ThreadNodeView tree from overview entries
fn build_thread_tree(
    root_id: &str,
    entries: &[&OverviewEntry],
    _entries_by_id: &HashMap<String, &OverviewEntry>,
) -> ThreadNodeView {
    // Build parent -> children map from references
    let mut children_map: HashMap<String, Vec<&OverviewEntry>> = HashMap::new();

    for entry in entries {
        let _msg_id = match entry.message_id() {
            Some(id) => id.to_string(),
            None => continue,
        };

        // Find direct parent from references (last reference is direct parent)
        let parent_id = if let Some(refs) = entry.references() {
            if refs.trim().is_empty() {
                None // Root message
            } else {
                refs.split_whitespace().last().map(|s| s.to_string())
            }
        } else {
            None
        };

        if let Some(parent) = parent_id {
            children_map.entry(parent).or_default().push(entry);
        }
    }

    // Build tree recursively from root
    build_node_from_entry(root_id, entries, &children_map)
}

/// Build a single node and its children
fn build_node_from_entry(
    msg_id: &str,
    entries: &[&OverviewEntry],
    children_map: &HashMap<String, Vec<&OverviewEntry>>,
) -> ThreadNodeView {
    // Find the entry for this message
    let entry = entries.iter().find(|e| e.message_id() == Some(msg_id));

    let article = entry.map(|e| overview_entry_to_article_view(e));

    // Build child nodes
    let mut replies: Vec<ThreadNodeView> = Vec::new();
    if let Some(children) = children_map.get(msg_id) {
        for child in children {
            if let Some(child_id) = child.message_id() {
                let child_node = build_node_from_entry(child_id, entries, children_map);
                replies.push(child_node);
            }
        }
    }

    // Compute descendant count
    let descendant_count: usize = replies.iter()
        .map(|r| 1 + r.descendant_count)
        .sum();

    ThreadNodeView {
        message_id: msg_id.to_string(),
        article,
        replies,
        descendant_count,
    }
}

/// Convert OverviewEntry to ArticleView
fn overview_entry_to_article_view(entry: &OverviewEntry) -> ArticleView {
    ArticleView {
        message_id: entry.message_id().unwrap_or("").to_string(),
        subject: entry.subject().unwrap_or(DEFAULT_SUBJECT).to_string(),
        from: entry.from().unwrap_or("").to_string(),
        date: entry.date().unwrap_or("").to_string(),
        body: None, // Overview doesn't include body
        headers: None,
    }
}

/// Find the latest date from overview entries
fn find_latest_date_overview(entries: &[&OverviewEntry]) -> Option<String> {
    use chrono::DateTime;

    let mut latest: Option<(String, DateTime<chrono::FixedOffset>)> = None;

    for entry in entries {
        if let Some(date_str) = entry.date() {
            if let Ok(parsed) = DateTime::parse_from_rfc2822(date_str) {
                if latest.is_none() || parsed > latest.as_ref().unwrap().1 {
                    latest = Some((date_str.to_string(), parsed));
                }
            }
        }
    }

    latest.map(|(s, _)| s)
}

/// Merge new articles into an existing thread cache.
///
/// Updates existing threads with new replies and creates new threads for
/// messages that do not belong to any existing thread.
pub fn merge_articles_into_threads(
    existing: &[ThreadView],
    new_entries: Vec<OverviewEntry>,
) -> Vec<ThreadView> {
    if new_entries.is_empty() {
        return existing.to_vec();
    }

    // Build lookup of existing threads by root message ID
    let mut threads_by_root: HashMap<String, ThreadView> = existing
        .iter()
        .map(|t| (t.root_message_id.clone(), t.clone()))
        .collect();

    // Also build a lookup of all known message IDs to their thread root
    let mut msg_to_root: HashMap<String, String> = HashMap::new();
    for thread in existing {
        collect_message_ids_to_root(&thread.root, &thread.root_message_id, &mut msg_to_root);
    }

    // Group new entries by which thread they belong to
    let mut updates_by_thread: HashMap<String, Vec<&OverviewEntry>> = HashMap::new();
    let mut new_roots: Vec<&OverviewEntry> = Vec::new();

    for entry in &new_entries {
        let msg_id = match entry.message_id() {
            Some(id) => id.to_string(),
            None => continue,
        };

        // Check if this message references a known message
        let thread_root = if let Some(refs) = entry.references() {
            refs.split_whitespace()
                .find_map(|ref_id| msg_to_root.get(ref_id).cloned())
        } else {
            None
        };

        if let Some(root_id) = thread_root {
            updates_by_thread.entry(root_id).or_default().push(entry);
            msg_to_root.insert(msg_id, updates_by_thread.keys().last().unwrap().clone());
        } else {
            // Check if this is a known root
            if threads_by_root.contains_key(&msg_id) {
                updates_by_thread.entry(msg_id.clone()).or_default().push(entry);
            } else {
                // New thread
                new_roots.push(entry);
            }
        }
    }

    // Update existing threads with new entries
    for (root_id, entries) in updates_by_thread {
        if let Some(thread) = threads_by_root.get_mut(&root_id) {
            // Add new entries to the thread
            for entry in &entries {
                if let Some(msg_id) = entry.message_id() {
                    let new_node = ThreadNodeView {
                        message_id: msg_id.to_string(),
                        article: Some(overview_entry_to_article_view(entry)),
                        replies: Vec::new(),
                        descendant_count: 0,
                    };

                    // Find parent in references and add as child
                    if let Some(refs) = entry.references() {
                        let parent_id = refs.split_whitespace().last();
                        if let Some(parent) = parent_id {
                            add_reply_to_node(&mut thread.root, parent, new_node);
                        }
                    }
                }
            }

            // Update article count and last post date
            thread.article_count += entries.len();
            if let last_date @ Some(_) = find_latest_date_overview(&entries) {
                thread.last_post_date = last_date;
            }
        }
    }

    // Build new threads from new roots
    let new_thread_entries: Vec<OverviewEntry> = new_roots.iter().map(|e| (*e).clone()).collect();
    let new_threads = build_threads_from_overview(new_thread_entries);

    // Combine existing (updated) and new threads
    let mut result: Vec<ThreadView> = threads_by_root.into_values().collect();
    result.extend(new_threads);

    result
}

/// Merge new articles into a single thread.
///
/// Filters entries to only those that reference message IDs already in the thread,
/// then adds them to the appropriate parent nodes.
pub fn merge_articles_into_thread(
    existing: &ThreadView,
    new_entries: Vec<OverviewEntry>,
) -> ThreadView {
    if new_entries.is_empty() {
        return existing.clone();
    }

    // Build set of all message IDs in the existing thread for fast lookup
    let known_ids = collect_all_message_ids(&existing.root);
    
    // Filter to only entries that reference a known message ID
    let relevant_entries: Vec<&OverviewEntry> = new_entries
        .iter()
        .filter(|entry| {
            if let Some(refs) = entry.references() {
                refs.split_whitespace().any(|ref_id| known_ids.contains(ref_id))
            } else {
                false
            }
        })
        .collect();

    if relevant_entries.is_empty() {
        return existing.clone();
    }

    // Clone the thread and add new entries
    let mut updated = existing.clone();
    
    for entry in &relevant_entries {
        if let Some(msg_id) = entry.message_id() {
            // Skip if already in thread
            if known_ids.contains(msg_id) {
                continue;
            }
            
            let new_node = ThreadNodeView {
                message_id: msg_id.to_string(),
                article: Some(overview_entry_to_article_view(entry)),
                replies: Vec::new(),
                descendant_count: 0,
            };

            // Find parent in references and add as child
            if let Some(refs) = entry.references() {
                if let Some(parent_id) = refs.split_whitespace().last() {
                    add_reply_to_node(&mut updated.root, parent_id, new_node);
                }
            }
        }
    }

    // Update article count and last post date
    updated.article_count += relevant_entries.len();
    if let Some(latest) = find_latest_date_overview(&relevant_entries) {
        updated.last_post_date = Some(latest);
    }

    updated
}

/// Collect all message IDs in a thread tree and map them to the root
fn collect_message_ids_to_root(
    node: &ThreadNodeView,
    root_id: &str,
    map: &mut HashMap<String, String>,
) {
    map.insert(node.message_id.clone(), root_id.to_string());
    for reply in &node.replies {
        collect_message_ids_to_root(reply, root_id, map);
    }
}

/// Collect all message IDs in a thread tree into a HashSet for efficient lookup
fn collect_all_message_ids(node: &ThreadNodeView) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    let mut stack = vec![node];
    
    while let Some(n) = stack.pop() {
        ids.insert(n.message_id.clone());
        for reply in &n.replies {
            stack.push(reply);
        }
    }
    
    ids
}

/// Add a reply node to the appropriate parent in the tree
fn add_reply_to_node(
    node: &mut ThreadNodeView,
    parent_id: &str,
    new_reply: ThreadNodeView,
) -> bool {
    if node.message_id == parent_id {
        node.replies.push(new_reply);
        // Update descendant count
        node.descendant_count += 1;
        return true;
    }

    for reply in &mut node.replies {
        if add_reply_to_node(reply, parent_id, new_reply.clone()) {
            // Update ancestor's descendant count
            node.descendant_count += 1;
            return true;
        }
    }

    false
}

/// Raw article data collected from NNTP HDR commands before parsing.
#[derive(Debug, Clone)]
pub struct HdrArticleData {
    pub message_id: String,
    pub references: Option<String>,
    pub subject: String,
    pub from: String,
    pub date: String,
}

/// Build a thread list from NNTP HDR command response data.
///
/// Uses the References header to reconstruct thread structure.
pub fn build_threads_from_hdr(articles: Vec<HdrArticleData>) -> Vec<ThreadView> {
    if articles.is_empty() {
        return Vec::new();
    }

    // Build a map of message_id -> HdrArticleData for quick lookup
    let mut articles_by_id: HashMap<String, &HdrArticleData> = HashMap::new();
    for article in &articles {
        articles_by_id.insert(article.message_id.clone(), article);
    }

    // Group articles by thread root (first message in references chain, or self if no references)
    let mut threads_map: HashMap<String, Vec<&HdrArticleData>> = HashMap::new();

    for article in &articles {
        // Parse references to find thread root
        let root_id = if let Some(refs) = &article.references {
            if refs.trim().is_empty() {
                // No references - this is a root message
                article.message_id.clone()
            } else {
                // First reference is the thread root
                refs.split_whitespace()
                    .next()
                    .unwrap_or(&article.message_id)
                    .to_string()
            }
        } else {
            // No references field - this is a root message
            article.message_id.clone()
        };

        threads_map.entry(root_id).or_default().push(article);
    }

    // Build ThreadView for each thread
    let mut thread_views: Vec<ThreadView> = Vec::new();

    for (root_id, thread_articles) in threads_map {
        // Find the actual root article (might not be in our articles if it's older/expired)
        let root_article = thread_articles
            .iter()
            .find(|a| a.message_id == root_id);

        // Get subject from root article if available, otherwise from first available article
        let subject = root_article
            .or_else(|| thread_articles.first())
            .map(|a| a.subject.clone())
            .unwrap_or_else(|| DEFAULT_SUBJECT.to_string());

        // Build the tree structure using original root_id
        // If root article is missing, build_node_from_hdr will create a node with article: None
        let root_node = build_thread_tree_hdr(&root_id, &thread_articles, &articles_by_id);
        let last_post_date = find_latest_date_hdr(&thread_articles);

        thread_views.push(ThreadView {
            subject,
            // Always use original root_id so thread can be found even if root article is missing
            root_message_id: root_id,
            article_count: thread_articles.len(),
            root: root_node,
            last_post_date,
        });
    }

    thread_views
}

/// Build a ThreadNodeView tree from HDR article data
fn build_thread_tree_hdr(
    root_id: &str,
    articles: &[&HdrArticleData],
    _articles_by_id: &HashMap<String, &HdrArticleData>,
) -> ThreadNodeView {
    // Build parent -> children map from references
    let mut children_map: HashMap<String, Vec<&HdrArticleData>> = HashMap::new();

    for article in articles {
        // Find direct parent from references (last reference is direct parent)
        let parent_id = if let Some(refs) = &article.references {
            if refs.trim().is_empty() {
                None // Root message
            } else {
                refs.split_whitespace().last().map(|s| s.to_string())
            }
        } else {
            None
        };

        if let Some(parent) = parent_id {
            children_map.entry(parent).or_default().push(article);
        }
    }

    // Build tree recursively from root
    build_node_from_hdr(root_id, articles, &children_map)
}

/// Build a single node and its children from HDR data
fn build_node_from_hdr(
    msg_id: &str,
    articles: &[&HdrArticleData],
    children_map: &HashMap<String, Vec<&HdrArticleData>>,
) -> ThreadNodeView {
    // Find the article for this message
    let article = articles.iter().find(|a| a.message_id == msg_id);

    let article_view = article.map(|a| ArticleView {
        message_id: a.message_id.clone(),
        subject: a.subject.clone(),
        from: a.from.clone(),
        date: a.date.clone(),
        body: None, // HDR doesn't include body
        headers: None,
    });

    // Build child nodes
    let mut replies: Vec<ThreadNodeView> = Vec::new();
    if let Some(children) = children_map.get(msg_id) {
        for child in children {
            let child_node = build_node_from_hdr(&child.message_id, articles, children_map);
            replies.push(child_node);
        }
    }

    // Compute descendant count
    let descendant_count: usize = replies.iter().map(|r| 1 + r.descendant_count).sum();

    ThreadNodeView {
        message_id: msg_id.to_string(),
        article: article_view,
        replies,
        descendant_count,
    }
}

/// Find the latest date from HDR article data
fn find_latest_date_hdr(articles: &[&HdrArticleData]) -> Option<String> {
    use chrono::DateTime;

    let mut latest: Option<(String, DateTime<chrono::FixedOffset>)> = None;

    for article in articles {
        if let Ok(parsed) = DateTime::parse_from_rfc2822(&article.date) {
            if latest.is_none() || parsed > latest.as_ref().unwrap().1 {
                latest = Some((article.date.clone(), parsed));
            }
        }
    }

    latest.map(|(s, _)| s)
}
