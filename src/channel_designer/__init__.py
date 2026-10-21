"""``channel_designer`` is the old name of :mod:`owd` (Open Waterway Designer).

This shim keeps ``import channel_designer`` working for one release. It gives a
``DeprecationWarning`` and then *is* :mod:`owd`: ``channel_designer``, and every submodule
(``channel_designer.rules`` and so on), are the same module objects as their ``owd`` twins.
It will be removed in the release after the rename.
"""
import importlib
import importlib.abc
import importlib.machinery
import sys
import warnings

import owd

_OLD = "channel_designer"
_NEW = "owd"

warnings.warn(
    "channel_designer is now owd (Open Waterway Designer): use `import owd`. "
    "The channel_designer name will be removed in a later release.",
    DeprecationWarning,
    stacklevel=2,
)


class _Alias(importlib.abc.Loader):
    """Hands out the module ``owd.<x>`` as ``channel_designer.<x>`` and leaves its own spec alone."""

    def __init__(self, target):
        self._target = target
        self._spec = target.__spec__

    def create_module(self, spec):
        return self._target

    def exec_module(self, module):
        # The import system has just written the alias spec onto the shared module. Put the real one back.
        module.__spec__ = self._spec


class _Finder(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if not fullname.startswith(_OLD + "."):
            return None
        real = importlib.import_module(_NEW + fullname[len(_OLD):])
        return importlib.machinery.ModuleSpec(fullname, _Alias(real), is_package=hasattr(real, "__path__"))


if not any(isinstance(f, _Finder) for f in sys.meta_path):
    sys.meta_path.insert(0, _Finder())

# `channel_designer` itself becomes `owd`.
sys.modules[_OLD] = owd
