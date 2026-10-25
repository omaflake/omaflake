# No catalog

The default of the flake input `omaflake-catalog`: empty, so the flake evaluates on its own (`nix flake check`).
omaflake replaces this input whenever it evaluates or builds – with a catalog-shaped folder: `recipes/`, `modules/`
and `keymaps/` as in omaflake/catalog (spec §11.3, plan B142), either a catalog checkout or the trees omaflake stages
for its builds (`recipes.json`).
