"""A page of the web as text that a model reads."""

from columbina.web.page import text_of

HERE = "https://example.com/docs/page.html"


def test_what_a_reader_sees_is_kept_in_the_shape_of_markdown() -> None:
    html = """<!doctype html><html><head><title> The  moon </title>
    <style>body { color: red }</style><script>var x = "<p>no</p>";</script></head>
    <body><nav><a href="/">Home</a></nav>
    <h1>Phases</h1><p>The moon has   <b>phases</b>.<br>It turns.</p>
    <h3>List</h3><ul><li>new</li><li>full<ul><li>bright</li></ul></li></ul>
    <ol><li>one</li></ol><hr>
    <pre>def f():
    return 1</pre><p>Call <code>f()</code>.</p>
    <p><a href="other.html">Other page</a> and <a href="https://x.org/a">https://x.org/a</a>
    and <a href="javascript:alert(1)">a script</a> and <a>nowhere</a>
    and <a href="mailto:a@b.c"></a></p>
    <img src="a.png" alt=" A  picture "><img src="b.png"><img src="c.png"/>
    <noscript>Turn on scripts</noscript><template><p>later</p></template>
    <svg><text>drawn</text></svg><textarea>typed</textarea><select><option>x</option></select>
    <iframe src="x"></iframe><br/><div/>
    <table><tr><th>a</th><td>b</td></tr></table>
    </body></html>"""

    page = text_of(html, HERE)

    assert page.title == "The moon"
    assert page.text == (
        "[Home](https://example.com/)\n\n"
        "# Phases\n\n"
        "The moon has phases.\nIt turns.\n\n"
        "### List\n\n"
        "- new\n\n"
        "- full\n\n"
        "  - bright\n\n"
        "- one\n\n"
        "---\n\n"
        "```\ndef f():\n    return 1\n```\n\n"
        "Call `f()`.\n\n"
        "[Other page](https://example.com/docs/other.html) and https://x.org/a and a script "
        "and nowhere and mailto:a@b.c\n\n"
        "[A picture]\n\n"
        "a\n\n"
        "b"
    )


def test_a_page_without_a_title_or_text_has_neither() -> None:
    page = text_of("<html><body><script>x()</script></body></html>", HERE)

    assert page.title == ""
    assert page.text == ""


def test_blocks_inside_a_link_stay_on_its_line() -> None:
    page = text_of('<p><a href="/card"><div>A</div><div>card</div></a></p>', HERE)

    assert page.text == "[A card](https://example.com/card)"


def test_ends_without_beginnings_change_nothing() -> None:
    page = text_of("</script></ul></pre></a><p>still</p></code>", HERE)

    assert page.text == "still"
