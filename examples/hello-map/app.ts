import type { Answer, Result, Sources } from './app.contract.d.ts';

export const grants = '';

const places = [
  { id: 'grossmuenster', name: 'Grossmünster', coordinates: [8.5440, 47.3701] },
  { id: 'fraumuenster', name: 'Fraumünster', coordinates: [8.5414, 47.3697] },
  { id: 'hb', name: 'Zürich HB', coordinates: [8.5402, 47.3779] },
  { id: 'eth', name: 'ETH Zürich', coordinates: [8.5481, 47.3763] },
  { id: 'opera', name: 'Opernhaus', coordinates: [8.5466, 47.3653] },
];

const walk = ['hb', 'eth', 'grossmuenster', 'fraumuenster', 'opera'].map(
  (id) => places.find((place) => place.id === id)!.coordinates,
);

// The map's own sources and layers, in the MapLibre Style Specification's terms.
const content = (chosen: string) => ({
  hitPadding: 8,
  images: {
    pin: { src: 'assets/pin.png', pixelRatio: 2 },
    'pin-selected': { src: 'assets/pin-selected.png', pixelRatio: 2 },
  },
  sources: {
    walk: { type: 'geojson', data: { type: 'Feature', properties: {}, geometry: { type: 'LineString', coordinates: walk } } },
    places: {
      type: 'geojson',
      data: {
        type: 'FeatureCollection',
        features: places.map(({ id, name, coordinates }) => ({
          type: 'Feature',
          properties: { id, name },
          geometry: { type: 'Point', coordinates },
        })),
      },
    },
  },
  layers: [
    {
      id: 'walk',
      type: 'line',
      source: 'walk',
      beforeType: 'symbol',
      layout: { 'line-join': 'round', 'line-cap': 'round' },
      paint: { 'line-color': '#1f6feb', 'line-width': 5, 'line-opacity': 0.5 },
    },
    {
      id: 'places',
      type: 'symbol',
      source: 'places',
      layout: {
        'icon-image': ['case', ['==', ['get', 'id'], chosen], 'pin-selected', 'pin'],
        'icon-anchor': 'bottom',
        'icon-allow-overlap': true,
        'text-field': ['get', 'name'],
        'text-font': ['Noto Sans Regular'],
        'text-size': 13,
        'text-anchor': 'top',
        'text-offset': [0, 0.2],
        'text-optional': true,
      },
      paint: { 'text-color': '#1c2024', 'text-halo-color': '#ffffff', 'text-halo-width': 1.5 },
    },
  ],
});

const sources: Sources = {
  mapContent: ([chosen]): Result<'mapContent'> => ({ json: JSON.stringify(content(chosen)) }),
};
export const answer: Answer = (source, args, store, storage, native) =>
  sources[source](args, store, storage, native);
