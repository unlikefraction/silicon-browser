//! External-service boundaries.
//!
//! Domain code depends on the traits here for Browser Use and TinyFish.
//! Historical upload receipt DTOs remain readable without a delivery-service client.

mod browser;
mod error;
mod pool;
mod proxy;
mod search;
#[cfg(test)]
pub(crate) mod test_http;

pub use browser::{
    BrowserProvider, BrowserUseV3, CreateBrowserProfile, ProviderAccountLimits, ProviderBrowserSession,
    ProviderProfile, StartBrowser, UpdateBrowserProfile,
};
pub use error::{ProviderError, ProviderResult};
pub use pool::{
    ActorSearchProvider, FAIR_SEARCH_QUEUE_CAPACITY, FairSearchPool, TINYFISH_FETCH_URLS_PER_MINUTE,
    TINYFISH_SEARCH_REQUESTS_PER_MINUTE,
};
pub use proxy::{
    BROWSER_USE_PROXY_LOCATIONS, BROWSER_USE_PROXY_SCHEMA_SNAPSHOT, ProviderProxyLocation, is_proxy_location,
    proxy_locations,
};
pub use search::{
    FetchFormat, FetchItem, FetchRequest, FetchResponse, FetchStatus, SearchProvider, SearchRequest, SearchResponse,
    SearchResult, SearchType, TinyFish,
};
