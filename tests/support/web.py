"""A web for tests: answers of HTTP that a mock transport gives, and a page of results."""

from collections.abc import Callable
from typing import Any

import httpx

Answer = Callable[[httpx.Request], httpx.Response]


def responding(status: int = 200, **given: Any) -> Answer:
    """Return what answers every request with the same response."""

    def answer(request: httpx.Request) -> httpx.Response:
        return httpx.Response(status, **given)

    return answer


DUCKDUCKGO_PAGE = """<html><body><div class="serp__results">
<div class="result results_links results_links_deep result--ad">
  <a class="result__a" href="https://duckduckgo.com/y.js?ad=1">An ad</a>
</div>
<div class="result results_links results_links_deep web-result ">
  <div class="links_main links_deep result__body">
  <h2><a rel="nofollow" class="result__a"
     href="https://docs.python.org/3/">Python &amp; <b>docs</b></a></h2>
  <a class="result__snippet" href="https://docs.python.org/3/">The   <b>official</b> docs.</a>
  </div>
</div>
<div class="result results_links web-result ">
  <a class="result__a"
     href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.org%2Fa%3Fb%3D1&amp;rut=x">Round</a>
</div>
<div class="result results_links web-result "><p>no link</p></div>
<div class="result results_links web-result ">
  <a class="result__a" href="https://third.example/">Third</a>
</div>
</div></body></html>"""
