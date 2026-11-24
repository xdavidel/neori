//! Tests for `neori.util`, run as Lua assertions in a config.

use std::path::Path;

use neori_config::Config;

/// Runs `code` as a config in `dir`/config.lua, panicking with the Lua error if an assertion
/// fails.
#[track_caller]
fn run_in(dir: &Path, code: &str) {
    let prelude = r#"
        local function same(a, b, path)
            path = path or "value"
            if type(a) ~= "table" or type(b) ~= "table" then
                if a ~= b then
                    error(path .. ": " .. tostring(a) .. " ~= " .. tostring(b), 2)
                end
                return
            end
            for k, v in pairs(a) do same(v, b[k], path .. "." .. tostring(k)) end
            for k, v in pairs(b) do same(a[k], v, path .. "." .. tostring(k)) end
        end
        local util = neori.util
    "#;
    let result = Config::parse_with_host(&dir.join("config.lua"), &format!("{prelude}\n{code}"));
    if let Err(err) = result.config {
        panic!("{err:?}");
    }
}

#[track_caller]
fn run(code: &str) {
    run_in(Path::new("/nonexistent"), code);
}

#[test]
fn table() {
    run(r#"
        local t = util.table
        same(t.join({ 1, 2, a = "x" }, nil, { 3, a = "y", b = "z" }), { 1, 2, 3, a = "y", b = "z" })

        local target = { a = 1 }
        assert(t.crush(target, { a = 2, b = 3 }) == target)
        same(target, { a = 2, b = 3 })

        same(t.from_sparse({ [1] = "a", [5] = "b", [3] = "c", x = "d" }), { "a", "c", "b" })
        assert(t.hasitem({ "a", "b", k = "c" }, "b") == 2)
        assert(t.hasitem({ "a", "b", k = "c" }, "c") == "k")
        assert(t.hasitem({ "a" }, "z") == nil)

        local even = function(_, v) return v % 2 == 0 end
        same(t.find_keys({ 1, 2, 3, 4, 6 }, even, true), { 2, 4, 5 })
        -- `max` stops the search (cwc's copy ignored it).
        same(t.find_keys({ 1, 2, 3, 4, 6 }, even, true, 2), { 2, 4 })
        assert(t.find_keys({ 1, 3 }, even) == nil)
        assert(t.find_first_key({ 1, 3, 4, 6 }, even, true) == 3)

        same(t.keys({ b = 1, a = 2, c = 3 }), { "a", "b", "c" })
        assert(t.count_keys({ 1, 2, a = 3 }) == 3)
        same(t.keys_filter({ a = 1, b = "s", c = 2, d = {} }, "number", "table"), { "a", "c", "d" })
        same(t.reverse({ 1, 2, 3, k = "v" }), { 3, 2, 1, k = "v" })

        local nested = { a = { b = 1 } }
        local deep, shallow = t.clone(nested), t.clone(nested, false)
        same(deep, nested)
        assert(deep.a ~= nested.a and shallow.a == nested.a)

        assert(t.cycle_value({ "a", "b", "c" }, "c") == "a")
        assert(t.cycle_value({ "a", "b", "c" }, "a", -1) == "c")
        local v, k = t.cycle_value({ 1, 2, 3, 4 }, 1, 1, function(x) return x > 2 end)
        assert(v == 3 and k == 3)
        assert(t.cycle_value({ "a" }, "z") == nil)

        local seen = {}
        for item in t.iterate({ 1, 2, 3, 4 }, function(x) return x ~= 3 end, 3) do
            seen[#seen + 1] = item
        end
        same(seen, { 4, 1, 2 })

        local list = { 1 }
        assert(t.merge(list, { 2, 3, k = "ignored" }) == list)
        same(list, { 1, 2, 3 })

        local id = function(e) return e.id end
        local old = { { id = 1, v = "a" }, { id = 2, v = "b" } }
        local out, added, removed, updated = t.diff_merge(
            old, { { id = 2, v = "B" }, { id = 3, v = "c" } }, id, t.crush)
        assert(out == old)
        same(out, { { id = 2, v = "B" }, { id = 3, v = "c" } })
        same(added, { { id = 3, v = "c" } })
        same(removed, { { id = 1, v = "a" } })
        same(updated, { { id = 2, v = "B" } })

        same(t.map(function(x) return x * 2 end, { 1, 2, a = 3 }), { 2, 4, a = 6 })
    "#);
}

#[test]
fn string() {
    run(r#"
        local s = util.string
        assert(s.xml_escape([[<a href="x">'&'</a>]]) == "&lt;a href=&quot;x&quot;&gt;&apos;&amp;&apos;&lt;/a&gt;")
        assert(s.xml_unescape("&lt;b&gt; &amp; &nbsp;") == "<b> &  ")
        assert(s.linecount("a\nb\nc") == 3)
        assert(s.linecount("") == 1)
        assert(s.linewrap("aaa bbb ccc ddd", 7) == "aaa bbb\nccc ddd")
        assert(s.linewrap("aaa bbb ccc", 3, 2) == "aaa\n  bbb\n  ccc")
        assert(s.quote_pattern("a.b*c") == "a%.b%*c")
        assert(("Firefox"):match(s.query_to_pattern("fire")))
        same(s.split("a,b,,c", ","), { "a", "b", "c" })
        same(s.split(",a,", ","), { "", "a", "" })
        same(s.split("abc", ","), { "abc" })
        same(s.split("a\nb"), { "a", "b" })
        same(s.psplit("a, b,c", ",%s*"), { "a", "b", "c" })
        same(s.psplit("abc", ""), { "a", "b", "c" })
        assert(s.startswith("neori", "neo") and not s.startswith("neori", "ori"))
        assert(s.endswith("neori", "ori") and s.endswith("neori", "") and not s.endswith(nil, "x"))
    "#);
}

#[test]
fn math() {
    run(r#"
        local m = util.math
        assert(m.round(1.4) == 1 and m.round(1.5) == 2 and m.round(-1.5) == -1)
        assert(m.sign(-3) == -1 and m.sign(0) == 0 and m.sign(2.5) == 1)
        assert(m.cycle(3, 4) == 1 and m.cycle(3, 3) == 3 and m.cycle(3, 0) == 3 and m.cycle(3, -1) == 2)
        assert(m.cycle(0, 1) == nil)
        local subsets = m.subsets({ 1, 2, 3 })
        assert(#subsets == 8)
        same(subsets[1], {})
        same(subsets[8], { 1, 2, 3 })
    "#);
}

#[test]
fn geometry() {
    run(r#"
        local g = util.geometry
        local a = { x = 0, y = 0, width = 100, height = 100 }
        local right = { x = 200, y = 0, width = 100, height = 100 }
        local below = { x = 0, y = 150, width = 100, height = 100 }
        local list = { left = a, right = right, below = below }

        assert(g.square_distance(a, 50, 50) == 0)
        assert(g.square_distance(a, 103, 104) == 4 * 4 + 5 * 5)
        assert(g.closest_by_coord(list, 190, 10) == "right")
        assert(g.by_coord(list, 50, 200) == "below")
        assert(g.by_coord(list, 150, 50) == nil)

        assert(g.in_direction("right", list, a) == "right")
        assert(g.in_direction("down", list, a) == "below")
        assert(g.in_direction("left", list, a) == nil)
        assert(g.in_direction("up", { a, below }, below) == 1)

        assert(g.are_equal(a, { x = 0, y = 0, width = 100, height = 100 }))
        assert(not g.are_equal(a, right))
        assert(g.is_inside({ x = 10, y = 10, width = 20, height = 20 }, a))
        assert(not g.is_inside(a, { x = 10, y = 10, width = 20, height = 20 }))
        assert(g.intersects(a, { x = 50, y = 50, width = 100, height = 100 }))
        assert(not g.intersects(a, right))
        same(g.intersection(a, { x = 50, y = 60, width = 100, height = 100 }),
            { x = 50, y = 60, width = 50, height = 40 })
        same(g.intersection(a, right), { x = 200, y = 0, width = 0, height = 0 })

        -- Taking the middle out of a 300 x 100 strip leaves its left and right parts.
        local areas = g.area_remove({ { x = 0, y = 0, width = 300, height = 100 } },
            { x = 100, y = 0, width = 100, height = 100 })
        same(areas, {
            { x = 0, y = 0, width = 100, height = 100 },
            { x = 200, y = 0, width = 100, height = 100 },
        })
    "#);
}

#[test]
fn dump() {
    run(r#"
        assert(util.dump(5) == "5 (number)")
        assert(util.dump("x", "name") == "name : x (string)")
        local text = util.dump({ a = { b = true } })
        assert(text:match("^table: 0x%x+\n  a : table: 0x%x+\n    b : true %(boolean%)$"), text)
        assert(util.dump({ a = {} }, nil, 1):match("a : table: 0x%x+ %(table%) %[…%]$"))
    "#);
}

#[test]
fn fs() {
    let dir = std::env::temp_dir().join(format!("neori-util-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("file"), "x").unwrap();

    run_in(
        &dir,
        &format!(
            r#"
        local fs = util.fs
        local dir = {dir:?}
        assert(fs.config_dir() == dir .. "/", fs.config_dir())
        for _, name in ipairs {{ "xdg_config_home", "xdg_cache_home", "xdg_data_home" }} do
            assert(fs[name]():sub(-1) == "/", name)
        end
        assert(#fs.xdg_data_dirs() > 0 and fs.xdg_data_dirs()[1]:sub(-1) == "/")

        assert(fs.file_readable(dir .. "/file"))
        assert(not fs.file_readable(dir))
        assert(not fs.file_readable(dir .. "/missing"))
        assert(not fs.file_executable(dir .. "/file"))
        assert(fs.file_executable("/bin/sh"))
        assert(fs.dir_readable(dir) and fs.dir_writable(dir) and fs.is_dir(dir))
        assert(not fs.dir_readable(dir .. "/file") and not fs.is_dir(dir .. "/file"))

        assert(fs.mkdir(dir .. "/a/b"))
        assert(fs.is_dir(dir .. "/a/b"))
        assert(fs.mkdir(dir .. "/a/b"), "an existing directory counts as made")
        local ok, err = fs.mkdir(dir .. "/file/c")
        assert(not ok and err:match("/file/c: "), err)
        "#
        ),
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
