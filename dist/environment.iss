[Code]
{ Adds and removes a folder from the current user's PATH (HKCU\Environment),
  which needs no administrator rights. Comparisons ignore case. }

const EnvironmentKey = 'Environment';

procedure EnvAddPath(Path: string);
var
  Paths: string;
begin
  { Retrieve current path (use empty string if entry not exists) }
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths) then
    Paths := '';

  { Skip if already in PATH }
  if Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';') > 0 then
    exit;

  { Append, with exactly one separator }
  if Paths = '' then
    Paths := Path
  else if Copy(Paths, Length(Paths), 1) = ';' then
    Paths := Paths + Path
  else
    Paths := Paths + ';' + Path;

  { Overwrite (or create if missing); an existing REG_EXPAND_SZ keeps its type }
  if RegWriteStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths) then
    Log(Format('The [%s] added to PATH: [%s]', [Path, Paths]))
  else
    Log(Format('Error while adding the [%s] to PATH: [%s]', [Path, Paths]));
end;

procedure EnvRemovePath(Path: string);
var
  Paths: string;
  P: Integer;
begin
  { Skip if registry entry not exists }
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths) then
    exit;

  { Work on ';' + Paths + ';' so every entry is surrounded by separators }
  Paths := ';' + Paths + ';';
  P := Pos(';' + Uppercase(Path) + ';', Uppercase(Paths));
  if P = 0 then
    exit;

  { Remove ';' + Path, keeping the separator after it, then drop the padding }
  Delete(Paths, P, Length(Path) + 1);
  Paths := Copy(Paths, 2, Length(Paths) - 2);

  if RegWriteStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', Paths) then
    Log(Format('The [%s] removed from PATH: [%s]', [Path, Paths]))
  else
    Log(Format('Error while removing the [%s] from PATH: [%s]', [Path, Paths]));
end;
