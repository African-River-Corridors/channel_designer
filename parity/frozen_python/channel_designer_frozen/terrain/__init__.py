"""Terrain along a route: long profiles over any terrain sampler (pure).

``terrain.profile`` samples any terrain sampler ``terrain(xs, ys) -> z`` — the contract the volume
integrators already use — along a line. Fetching elevation data is out of scope here (ROADMAP: spec 0002,
Copernicus GLO-30); a ``volumes.raster.RasterTerrain`` over a local GeoTIFF is one such sampler.
"""
from .profile import LineProfile, line_profile

__all__ = ["LineProfile", "line_profile"]
