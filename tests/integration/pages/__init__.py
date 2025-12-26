"""Page Object Model for integration tests."""

from .article import ArticlePage
from .base import BasePage
from .browse import BrowsePage
from .compose import ComposePage
from .dex import DexLoginPage
from .group import GroupPage
from .home import HomePage
from .thread import ThreadPage

__all__ = [
    "BasePage",
    "HomePage",
    "BrowsePage",
    "GroupPage",
    "ThreadPage",
    "ArticlePage",
    "ComposePage",
    "DexLoginPage",
]
