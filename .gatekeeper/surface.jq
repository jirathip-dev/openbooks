def bare: walk(if type == "object" then del(.id, .has_body, .impls, .items, .fields, .variants, .implementations, .tuple) else . end);
.index as $ix
| def item($p; $id):
    $ix[$id | tostring] as $it
    | select($it != null)
    | ($it.inner | keys[0]) as $k
    | (if $p == "" then $it.name else "\($p)::\($it.name)" end) as $q
    | if $k == "module" then "module \($q)", ($it.inner.module.items[] | item($q; .))
      elif $k == "use" then "use \($p)::\($it.inner.use.name) \($it.inner.use.source)"
      elif $k == "impl" then
        if $it.inner.impl.blanket_impl != null then empty
        elif $it.inner.impl.trait == null then $it.inner.impl.items[] | item($p; .)
        else "impl \($p) \($it.inner.impl.trait | bare | tojson)" end
      else "\($k) \($q) \($it.inner[$k] | bare | tojson)",
        (([$it.inner[$k] | .kind?.plain?.fields?, .kind?.tuple?, .kind?.struct?.fields?, .fields?, .variants?, .items?, .impls?] | map(arrays) | add // [])[] | item($q; .))
      end;
item(""; .root)
