{
  nixpkgs,
  system,
}:
let
  inherit (nixpkgs) lib;
  pkgs = nixpkgs.legacyPackages.${system};
  marker = "__strictixRenamed";
  capture = message: value: if builtins.isBool value then value else { ${marker} = message; };
  aliases = import "${nixpkgs}/pkgs/top-level/aliases.nix" (
    lib
    // {
      warn = capture;
      warnOnInstantiate = capture;
    }
  ) pkgs { };
  forced = value: builtins.tryEval value;
  captured =
    value:
    let
      result = forced value;
    in
    if result.success && builtins.isAttrs result.value && result.value ? ${marker} then
      result.value.${marker}
    else
      null;
  parse =
    message: old:
    let
      groups = builtins.match "(.*)'([^']+)' has been renamed to '([^']+)'" message;
    in
    if groups != null && builtins.elemAt groups 1 == old then builtins.elemAt groups 2 else null;
  isSet =
    value:
    let
      result = forced (builtins.isAttrs value && !lib.isDerivation value);
    in
    result.success && result.value;
  members =
    name: set:
    let
      names = builtins.attrNames set;
      messages = map (member: captured set.${member}) names;
      whole = parse (builtins.head messages) name;
    in
    if names == [ ] || builtins.head messages == null then
      [ ]
    else if whole != null && builtins.all (message: message == builtins.head messages) messages then
      [
        {
          old = name;
          new = whole;
        }
      ]
    else
      lib.concatLists (
        lib.zipListsWith (
          member: message:
          let
            new = if message == null then null else parse message "${name}.${member}";
          in
          lib.optional (new != null) {
            old = "${name}.${member}";
            inherit new;
          }
        ) names messages
      );
  entries = lib.concatMap (
    name:
    let
      value = aliases.${name};
      message = captured value;
      new = if message == null then null else parse message name;
    in
    if message != null then
      lib.optional (new != null) {
        old = name;
        inherit new;
      }
    else if isSet value then
      members name (forced value).value
    else
      [ ]
  ) (builtins.attrNames aliases);
  table = builtins.listToAttrs (
    map (entry: {
      name = entry.old;
      value = entry.new;
    }) entries
  );
  final = new: if table ? ${new} && table.${new} != new then final table.${new} else new;
  exists =
    new:
    let
      path = lib.splitString "." new;
    in
    lib.hasAttrByPath path pkgs && (forced (lib.getAttrFromPath path pkgs)).success;
  resolved = lib.filter (entry: entry.new != entry.old && exists entry.new) (
    map (entry: entry // { new = final entry.new; }) entries
  );
in
lib.concatMapStrings (entry: "${entry.old}\t${entry.new}\n") (
  lib.sort (a: b: a.old < b.old) resolved
)
