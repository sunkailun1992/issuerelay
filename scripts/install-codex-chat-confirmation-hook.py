#!/usr/bin/env python3
"""Install or remove the opt-in IssueRelay Codex prompt hook."""

import argparse
import json
import os
from pathlib import Path
import shlex
import stat
import tempfile


def hook_command(executable):
    path = str(executable)
    if any(ord(character) < 32 for character in path):
        raise ValueError("The IssueRelay executable path contains a control character")
    if os.name == "nt":
        # Codex runs command hooks through cmd.exe on Windows.
        if any(character in path for character in '"%!'):
            raise ValueError("The IssueRelay executable path cannot contain a quote, % or !")
        return f'"{path}" confirm-codex-input'
    return f"{shlex.quote(path)} confirm-codex-input"


def read_config(path):
    try:
        info = path.lstat()
    except FileNotFoundError:
        return None, None
    if not stat.S_ISREG(info.st_mode):
        raise ValueError("Codex hooks.json must be a regular file")
    return info, path.read_bytes()


def file_identity(info):
    if info is None:
        return None
    return (info.st_dev, info.st_ino, info.st_mode, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def unique_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Codex hooks.json contains duplicate object keys")
        result[key] = value
    return result


def invalid_constant(value):
    raise ValueError(f"Codex hooks.json contains a non-JSON value: {value}")


def install_or_remove(config_path, command, remove):
    original_info, original = read_config(config_path)
    config = {} if original is None else json.loads(
        original, object_pairs_hook=unique_keys, parse_constant=invalid_constant
    )
    if not isinstance(config, dict):
        raise ValueError("Codex hooks.json must contain a JSON object")

    hooks = config.setdefault("hooks", {})
    if not isinstance(hooks, dict):
        raise ValueError("Codex hooks.json 'hooks' must be an object")
    groups = hooks.get("UserPromptSubmit", [])
    if not isinstance(groups, list) or any(
        not isinstance(group, dict) or not isinstance(group.get("hooks"), list)
        for group in groups
    ):
        raise ValueError("Codex hooks.json contains an invalid UserPromptSubmit hook group")

    managed_hook = {"type": "command", "command": command, "timeout": 190}
    managed_group = {"hooks": [managed_hook]}
    if remove:
        updated_groups = [group for group in groups if group != managed_group]
        if groups == updated_groups:
            return False
        if updated_groups:
            hooks["UserPromptSubmit"] = updated_groups
        else:
            del hooks["UserPromptSubmit"]
    else:
        for group in groups:
            for hook in group["hooks"]:
                if (
                    isinstance(hook, dict)
                    and hook.get("type") == "command"
                    and hook.get("command") == command
                ):
                    return False
        hooks["UserPromptSubmit"] = [*groups, managed_group]

    content = (json.dumps(config, ensure_ascii=False, indent=2, allow_nan=False) + "\n").encode()
    with tempfile.NamedTemporaryFile(dir=config_path.parent, delete=False) as temp:
        temporary_path = Path(temp.name)
        try:
            temp.write(content)
            temp.flush()
            os.fsync(temp.fileno())
            temp.close()
            if original_info is not None:
                temporary_path.chmod(stat.S_IMODE(original_info.st_mode))
                current_info, current = read_config(config_path)
                if file_identity(current_info) != file_identity(original_info) or current != original:
                    raise ValueError("Codex hooks.json changed during installation; retry after saving it")
                os.replace(temporary_path, config_path)
            else:
                # Fails if another program creates hooks.json before the installer finishes.
                os.link(temporary_path, config_path)
        finally:
            temporary_path.unlink(missing_ok=True)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path, help="absolute path to the installed IssueRelay CLI")
    parser.add_argument("--remove", action="store_true", help="remove the matching installer entry")
    args = parser.parse_args()
    try:
        if not args.executable.is_absolute():
            raise ValueError("Pass the absolute path to the installed IssueRelay CLI")
        executable = args.executable.resolve(strict=not args.remove)
        if not args.remove and (not executable.is_file() or not os.access(executable, os.X_OK)):
            raise ValueError("The IssueRelay CLI must be an executable file")
        command = hook_command(executable)
        codex_home = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex").expanduser()
        config_path = codex_home / "hooks.json"
        if args.remove and not codex_home.exists():
            print("No matching installer entry")
            return
        codex_home.mkdir(parents=True, exist_ok=True)
        lock = codex_home / ".issuerelay-hook-install.lock"
        try:
            lock_handle = lock.open("x")
        except FileExistsError as error:
            raise ValueError("Another IssueRelay hook installer is using this Codex home") from error
        try:
            with lock_handle:
                changed = install_or_remove(config_path, command, args.remove)
        finally:
            lock.unlink()
    except (OSError, RuntimeError, ValueError) as error:
        parser.exit(1, f"{error}\n")

    if args.remove:
        print(f"Removed the IssueRelay hook from {config_path}" if changed else "No matching installer entry")
    elif changed:
        print(f"Added the IssueRelay hook to {config_path}. Review and trust it in Codex before use.")
    else:
        print(f"The IssueRelay command is already registered in {config_path}")


if __name__ == "__main__":
    main()
