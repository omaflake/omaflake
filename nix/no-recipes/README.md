# No recipes

The default of the flake input `omaflake-recipes`: empty, so the flake evaluates on its own (`nix flake check`).
omaflake replaces this input with the recipes of its sources whenever it evaluates or builds – a folder with
`<author>/<kind>/<name>/` recipe folders (omaflake/recipes, spec §11.3).
