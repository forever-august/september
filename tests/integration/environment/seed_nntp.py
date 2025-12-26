#!/usr/bin/env python3
"""
Seed the NNTP server with test data for integration tests.

This script connects to the Renews NNTP server and creates:
- Test newsgroups (via newgroup control messages)
- Test articles with threading (replies using References header)

Usage:
    python seed_nntp.py --host nntp --port 119
"""

import argparse
import nntplib
import socket
import sys
import time
from datetime import datetime, timedelta
from email.utils import formatdate
from typing import Optional
import uuid


def generate_message_id(domain: str = "test.integration") -> str:
    """Generate a unique Message-ID."""
    return f"<{uuid.uuid4()}.test@{domain}>"


def format_article(
    from_addr: str,
    newsgroup: str,
    subject: str,
    body: str,
    message_id: Optional[str] = None,
    references: Optional[str] = None,
    date: Optional[datetime] = None,
) -> tuple[str, bytes]:
    """
    Format an article for posting via NNTP.

    Returns:
        Tuple of (message_id, article_bytes) where article_bytes is the complete article.
    """
    if message_id is None:
        message_id = generate_message_id()

    if date is None:
        date = datetime.now()

    headers = [
        f"From: {from_addr}",
        f"Newsgroups: {newsgroup}",
        f"Subject: {subject}",
        f"Message-ID: {message_id}",
        f"Date: {formatdate(date.timestamp(), localtime=True)}",
        "User-Agent: SeptemberIntegrationTest/1.0",
    ]

    if references:
        headers.append(f"References: {references}")

    # Article format: headers, blank line, body
    lines = headers + ["", body]
    article = "\r\n".join(lines)

    return message_id, article.encode("utf-8")


def wait_for_server(host: str, port: int, timeout: int = 60) -> bool:
    """Wait for the NNTP server to be ready."""
    start = time.time()
    while time.time() - start < timeout:
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.settimeout(2)
            sock.connect((host, port))
            sock.close()
            return True
        except (socket.error, socket.timeout):
            print(f"Waiting for NNTP server at {host}:{port}...")
            time.sleep(2)
    return False


def create_newsgroup(nntp: nntplib.NNTP, group: str, description: str = "") -> bool:
    """
    Verify a newsgroup exists.

    Note: Groups are created via the renews admin CLI in docker-compose.
    This function just verifies they exist.
    """
    try:
        nntp.group(group)
        print(f"  Group {group} exists")
        return True
    except nntplib.NNTPTemporaryError as e:
        print(f"  Warning: Group {group} not found: {e}")
        return False


def seed_test_data(
    host: str, port: int, username: str = "testposter", password: str = "testpassword"
) -> None:
    """Seed the NNTP server with test data."""
    print(f"Connecting to NNTP server at {host}:{port}...")

    if not wait_for_server(host, port):
        print("ERROR: NNTP server not available")
        sys.exit(1)

    nntp = nntplib.NNTP(host, port)
    print(f"Connected: {nntp.getwelcome()}")

    # Authenticate
    print(f"Authenticating as {username}...")
    try:
        nntp.login(username, password)
        print("  Authentication successful")
    except nntplib.NNTPError as e:
        print(f"  Warning: Authentication failed: {e}")
        # Continue anyway - server might allow anonymous posting

    # Define test groups
    groups = [
        ("test.general", "General discussion for testing"),
        ("test.development", "Development topics"),
        ("test.announce", "Announcements"),
    ]

    print("\nCreating newsgroups...")
    for group, desc in groups:
        create_newsgroup(nntp, group, desc)

    # Give server time to process control messages
    time.sleep(1)

    # Define test users
    users = [
        "Alice Test <alice@example.com>",
        "Bob Developer <bob@example.com>",
        "Carol Admin <carol@example.com>",
    ]

    print("\nPosting test articles...")
    now = datetime.now()
    posted_ids: dict[str, str] = {}  # subject -> message_id for threading

    # test.general - Simple threads
    general_articles = [
        (
            "Welcome to test.general",
            "This is the first post in the general group.\n\nWelcome everyone!",
            None,
            0,
        ),
        (
            "Hello from Alice",
            "Just saying hello to everyone here.\n\nBest regards,\nAlice",
            None,
            1,
        ),
        (
            "Re: Welcome to test.general",
            "Thanks for the welcome!\n\nThis is a reply.",
            "Welcome to test.general",
            2,
        ),
        (
            "Question about testing",
            "How do we run the integration tests?\n\nThanks!",
            None,
            3,
        ),
        (
            "Re: Question about testing",
            "You can run them with pytest.\n\nSee the README for details.",
            "Question about testing",
            4,
        ),
    ]

    for subject, body, reply_to, hours_ago in general_articles:
        references = posted_ids.get(reply_to) if reply_to else None
        date = now - timedelta(hours=hours_ago)
        msg_id, lines = format_article(
            users[hours_ago % len(users)],
            "test.general",
            subject,
            body,
            references=references,
            date=date,
        )
        try:
            nntp.post(lines)
            posted_ids[subject.replace("Re: ", "")] = msg_id
            print(f"  Posted: {subject[:50]}...")
        except nntplib.NNTPError as e:
            print(f"  Failed to post '{subject}': {e}")

    # test.development - Deeper thread
    dev_thread_root = None
    dev_refs = ""
    dev_subjects = [
        (
            "New feature: OIDC support",
            "We've added OIDC authentication support.\n\nKey features:\n- Multiple providers\n- Session management\n- PKCE flow",
        ),
        ("Re: New feature: OIDC support", "This looks great! How do we configure it?"),
        (
            "Re: New feature: OIDC support",
            "Check the docs/oidc.md file for configuration details.",
        ),
        ("Re: New feature: OIDC support", "Thanks, that worked perfectly!"),
        (
            "Re: New feature: OIDC support",
            "Glad it helped. Let us know if you have more questions.",
        ),
    ]

    for i, (subject, body) in enumerate(dev_subjects):
        date = now - timedelta(hours=10 - i)
        msg_id, lines = format_article(
            users[i % len(users)],
            "test.development",
            subject,
            body,
            references=dev_refs if dev_refs else None,
            date=date,
        )
        try:
            nntp.post(lines)
            if dev_thread_root is None:
                dev_thread_root = msg_id
                dev_refs = msg_id
            else:
                dev_refs = f"{dev_refs} {msg_id}"
            print(f"  Posted: {subject[:50]}...")
        except nntplib.NNTPError as e:
            print(f"  Failed to post '{subject}': {e}")

    # More development articles (standalone)
    standalone_dev = [
        (
            "Bug fix: Cache invalidation",
            "Fixed an issue with cache TTL handling.\n\nThe cache now properly expires entries.",
        ),
        (
            "Performance improvements",
            "Optimized the thread fetching logic.\n\nShould be 2x faster now.",
        ),
        ("Documentation update", "Updated the README with new examples."),
    ]

    for i, (subject, body) in enumerate(standalone_dev):
        date = now - timedelta(hours=20 + i)
        msg_id, lines = format_article(
            users[i % len(users)],
            "test.development",
            subject,
            body,
            date=date,
        )
        try:
            nntp.post(lines)
            print(f"  Posted: {subject[:50]}...")
        except nntplib.NNTPError as e:
            print(f"  Failed to post '{subject}': {e}")

    # test.announce - Single post threads
    announcements = [
        (
            "Version 0.1.0 released",
            "We're excited to announce the first release!\n\nChangelog:\n- Initial NNTP support\n- Basic web interface",
        ),
        (
            "Maintenance window scheduled",
            "There will be a brief maintenance window on Saturday.\n\nExpected duration: 30 minutes.",
        ),
        ("New server added", "A new NNTP server has been added to the federation."),
    ]

    for i, (subject, body) in enumerate(announcements):
        date = now - timedelta(days=i + 1)
        msg_id, lines = format_article(
            "September Team <team@september.test>",
            "test.announce",
            subject,
            body,
            date=date,
        )
        try:
            nntp.post(lines)
            print(f"  Posted: {subject[:50]}...")
        except nntplib.NNTPError as e:
            print(f"  Failed to post '{subject}': {e}")

    # Verify data
    print("\nVerifying seeded data...")
    try:
        resp, count, first, last, name = nntp.group("test.general")
        print(f"  test.general: {count} articles")
    except nntplib.NNTPError as e:
        print(f"  test.general: error - {e}")

    try:
        resp, count, first, last, name = nntp.group("test.development")
        print(f"  test.development: {count} articles")
    except nntplib.NNTPError as e:
        print(f"  test.development: error - {e}")

    try:
        resp, count, first, last, name = nntp.group("test.announce")
        print(f"  test.announce: {count} articles")
    except nntplib.NNTPError as e:
        print(f"  test.announce: error - {e}")

    nntp.quit()
    print("\nSeeding complete!")


def main():
    parser = argparse.ArgumentParser(description="Seed NNTP server with test data")
    parser.add_argument("--host", default="localhost", help="NNTP server host")
    parser.add_argument("--port", type=int, default=119, help="NNTP server port")
    parser.add_argument("--username", default="testposter", help="NNTP username")
    parser.add_argument("--password", default="testpassword", help="NNTP password")
    args = parser.parse_args()

    seed_test_data(args.host, args.port, args.username, args.password)


if __name__ == "__main__":
    main()
