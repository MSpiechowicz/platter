import { describe, expect, it } from 'vitest';
import { namedWords, nameWords, placeHint, rulesNamedBy, scopeWords } from './mock-scope-words';

describe('the words of a scope name a rule by its title, its id or a name it goes by', () => {
  it('keeps the words that can name a rule, as stems, and drops the ones every rule has', () => {
    expect(scopeWords('docker images')).toEqual(['docker', 'image']);
    expect(scopeWords('the Steam shader cache')).toEqual(['steam', 'shader']);
    expect(scopeWords('node_modules')).toEqual(['node', 'module']);
    expect(scopeWords('my cache')).toEqual([]);
    expect(scopeWords('the files')).toEqual([]);
  });

  it('names the rules with the most words, and of those the closest fit', () => {
    const ids = (text: string): string[] => rulesNamedBy(text).map((rule) => rule.id);

    expect(ids('docker')).toEqual(['docker-data', 'docker-desktop-vm']);
    expect(ids('docker images')).toEqual(['docker-data', 'docker-desktop-vm']);
    expect(ids('Steam shader cache')).toEqual(['steam-shader-cache']);
    expect(ids('desktop')).toEqual(['desktop']);
    expect(ids('trash')).toEqual(['trash']);
    expect(ids('pacman')).toEqual(['pacman-cache']);
    expect(ids('pip')).toEqual(['python-cache']);
    expect(ids('Projekty')).toEqual([]);
    expect(ids('cache')).toEqual([]);
  });

  it('tells where a rule with one place looks, and says nothing of a rule that finds folders by name', () => {
    const [data, vm] = rulesNamedBy('docker');
    const [modules] = rulesNamedBy('node_modules');

    expect(data && placeHint(data)).toBe(
      '“Docker data” lives in /var/lib/docker, so scan a folder that holds it',
    );
    expect(vm && placeHint(vm)).toBe(
      '“Docker Desktop virtual machine” lives in ~/.docker/desktop/vms, so scan your home folder',
    );
    expect(modules && placeHint(modules)).toBeNull();
  });

  it('calls a missing place by the word that named its rule, else by a name it goes by', () => {
    const called = (text: string): string => namedWords(text, rulesNamedBy(text));

    expect(called('docker images')).toBe('docker');
    expect(called('the Docker containers')).toBe('Docker');
    expect(called('images')).toBe('images');
  });

  it('tries the words of a longer name longest first, and none for one word', () => {
    expect(nameWords('huggingface models')).toEqual(['huggingface', 'models']);
    expect(nameWords('the huggingface')).toEqual(['huggingface']);
    expect(nameWords('huggingface')).toEqual([]);
  });
});
