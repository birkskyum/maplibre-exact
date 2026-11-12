import type { Answer, Result, Sources } from './app.contract.d.ts';

export const grants = '';

const places = [
  { id: 'grossmuenster', name: 'Grossmünster', coordinates: [8.5440, 47.3701] },
  { id: 'fraumuenster', name: 'Fraumünster', coordinates: [8.5414, 47.3697] },
  { id: 'hb', name: 'Zürich HB', coordinates: [8.5402, 47.3779] },
  { id: 'eth', name: 'ETH Zürich', coordinates: [8.5481, 47.3763] },
  { id: 'opera', name: 'Opernhaus', coordinates: [8.5466, 47.3653] },
];

// The map's own sources and layers, in the MapLibre Style Specification's terms.
const content = (chosen: string) => ({
  hitPadding: 8,
  sources: {
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
      id: 'places',
      type: 'circle',
      source: 'places',
      paint: {
        'circle-radius': ['case', ['==', ['get', 'id'], chosen], 10, 7],
        'circle-color': ['case', ['==', ['get', 'id'], chosen], '#e5484d', '#1f6feb'],
        'circle-stroke-width': 2,
        'circle-stroke-color': '#ffffff',
      },
    },
    {
      id: 'place-names',
      type: 'symbol',
      source: 'places',
      layout: {
        'text-field': ['get', 'name'],
        'text-font': ['Noto Sans Regular'],
        'text-size': 13,
        'text-anchor': 'top',
        'text-offset': [0, 0.9],
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
