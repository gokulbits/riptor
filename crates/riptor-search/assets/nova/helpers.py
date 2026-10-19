"""Riptor's implementation of the nova3 search-plugin helper API.

retrieve_url / download_file / htmlentitydecode with the same signatures
plugins expect, plus safety limits: http(s) only, timeouts and size caps.
Written for Riptor; API-compatible with the nova3 helpers.
"""

import gzip
import html
import io
import os
import re
import sys
import tempfile
import urllib.parse
import urllib.request
import zlib

USER_AGENT = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0"
headers = {"User-Agent": USER_AGENT}


def getBrowserUserAgent():
    """Newer helper some plugins call."""
    return USER_AGENT


TIMEOUT = 15
MAX_PAGE = 8 * 1024 * 1024
MAX_TORRENT = 16 * 1024 * 1024


def _check_url(url):
    scheme = urllib.parse.urlsplit(url).scheme.lower()
    if scheme not in ("http", "https"):
        raise ValueError("only http(s) URLs are allowed: %r" % url)


class _HttpOnly(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, hdrs, newurl):
        _check_url(newurl)
        return super().redirect_request(req, fp, code, msg, hdrs, newurl)


def _opener(ssl_context=None):
    handlers = [_HttpOnly()]
    if ssl_context is not None:
        handlers.append(urllib.request.HTTPSHandler(context=ssl_context))
    return urllib.request.build_opener(*handlers)


def _read_capped(resp, cap):
    data = resp.read(cap + 1)
    if len(data) > cap:
        raise ValueError("response too large")
    enc = (resp.headers.get("Content-Encoding") or "").lower()
    if enc == "gzip" or data[:2] == b"\x1f\x8b":
        data = gzip.GzipFile(fileobj=io.BytesIO(data)).read(cap + 1)
    elif enc == "deflate":
        data = zlib.decompress(data)
    if len(data) > cap:
        raise ValueError("response too large")
    return data


def htmlentitydecode(s):
    return html.unescape(s)


def retrieve_url(url, custom_headers={}, request_data=None, ssl_context=None, unescape_html_entities=True):
    """Fetch a URL and return its body as text ('' on failure)."""
    try:
        _check_url(url)
        req = urllib.request.Request(url, data=request_data, headers={**headers, **dict(custom_headers)})
        with _opener(ssl_context).open(req, timeout=TIMEOUT) as resp:
            data = _read_capped(resp, MAX_PAGE)
            charset = resp.headers.get_content_charset() or "utf-8"
    except Exception as e:  # plugins expect '' rather than an exception
        sys.stderr.write("retrieve_url failed for %s: %s\n" % (url, e))
        return ""
    text = data.decode(charset, errors="replace")
    return html.unescape(text) if unescape_html_entities else text


def download_file(url, referer=None, ssl_context=None):
    """Download a .torrent to a temp file; returns 'path url'."""
    _check_url(url)
    h = dict(headers)
    if referer:
        h["Referer"] = referer
    req = urllib.request.Request(url, headers=h)
    with _opener(ssl_context).open(req, timeout=TIMEOUT) as resp:
        data = _read_capped(resp, MAX_TORRENT)
    fd, path = tempfile.mkstemp(suffix=".torrent")
    with os.fdopen(fd, "wb") as f:
        f.write(data)
    return path + " " + url


def enable_socks_proxy(enable):
    """Riptor does not route plugin traffic through a proxy."""
    return None


_tag_re = re.compile(r"<[^>]+>")


def strip_tags(s):
    return _tag_re.sub("", s)
