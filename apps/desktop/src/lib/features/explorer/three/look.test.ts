import { Color } from 'three';
import { describe, expect, it } from 'vitest';
import { COLUMN_MIX, COLUMN_MIX_LIGHT } from '../colors';
import { Look } from './look';

const DARK = new Color('#0a0e17');
const LIGHT = new Color('#eef1f6');

/** How light a colour is, 0 to 3. */
const lightness = (color: Color): number => color.r + color.g + color.b;

describe('Look', () => {
  it('mixes a column towards the page background, so a light page gives lighter columns', () => {
    const onDark = new Look('#d6739f', DARK, COLUMN_MIX);
    const onLight = new Look('#d6739f', LIGHT, COLUMN_MIX);
    const [darkFill, lightFill] = [onDark.fills[2], onLight.fills[2]] as [
      Look['fills'][number],
      Look['fills'][number],
    ];
    const [darkEdge, lightEdge] = [onDark.edges[1], onLight.edges[1]] as [
      Look['edges'][number],
      Look['edges'][number],
    ];

    expect(lightness(lightFill.color)).toBeGreaterThan(lightness(darkFill.color));
    expect(lightness(lightEdge.color)).toBeGreaterThan(lightness(darkEdge.color));
  });

  it('is painted again in place by retheme, and a column that is not selected goes towards the new background', () => {
    const look = new Look('#3ec5d8', DARK, COLUMN_MIX);
    const material = look.fills[0] as Look['fills'][number];

    look.paint(true, false);
    const dimmedOnDark = lightness(material.color);

    look.retheme(LIGHT, COLUMN_MIX_LIGHT);
    look.paint(true, false);

    expect(look.fills[0]).toBe(material);
    expect(lightness(material.color)).toBeGreaterThan(dimmedOnDark);

    look.paint(false, false);

    const fresh = new Look('#3ec5d8', LIGHT, COLUMN_MIX_LIGHT);

    expect(material.color.getHex()).toBe((fresh.fills[0] as Look['fills'][number]).color.getHex());
  });
});
