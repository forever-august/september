//! Main Leptos application component

use leptos::*;
use leptos_meta::*;
use leptos_router::*;

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/september.css"/>
        <Title text="September - HTTP to NNTP Bridge"/>

        <Router>
            <main>
                <Routes>
                    <Route path="" view=HomePage/>
                    <Route path="/groups" view=GroupsPage/>
                    <Route path="/groups/:group" view=GroupView/>
                    <Route path="/*any" view=NotFound/>
                </Routes>
            </main>
        </Router>
    }
}

/// Home page component
#[component]
fn HomePage() -> impl IntoView {
    view! {
        <div class="container">
            <h1>"September - HTTP to NNTP Bridge"</h1>
            <p>"Welcome to September, a bridge between HTTP and NNTP protocols."</p>
            <nav>
                <a href="/groups">"Browse Newsgroups"</a>
            </nav>
        </div>
    }
}

/// Newsgroups listing page
#[component]
fn GroupsPage() -> impl IntoView {
    view! {
        <div class="container">
            <h1>"Newsgroups"</h1>
            <p>"Select a newsgroup to browse:"</p>
            <div class="groups-list">
                <p>"Loading newsgroups..."</p>
                // TODO: Implement actual NNTP group fetching
            </div>
        </div>
    }
}

/// Individual newsgroup view
#[component]
fn GroupView() -> impl IntoView {
    let params = use_params_map();
    let group = move || params.with(|params| params.get("group").cloned().unwrap_or_default());

    view! {
        <div class="container">
            <h1>"Newsgroup: " {group}</h1>
            <p>"Articles in this newsgroup will be displayed here."</p>
            // TODO: Implement actual NNTP article fetching
        </div>
    }
}

/// 404 - Not Found
#[component]
fn NotFound() -> impl IntoView {
    // Set the status code for SSR
    let resp = use_context::<leptos_axum::ResponseOptions>();
    if let Some(resp) = resp {
        resp.set_status(axum::http::StatusCode::NOT_FOUND);
    }

    view! {
        <div class="container">
            <h1>"Not Found"</h1>
            <p>"The page you're looking for doesn't exist."</p>
            <a href="/">"Go Home"</a>
        </div>
    }
}
