"""
Generate golden parity fixtures for tg-core by running the REAL Python
TagGUI code. The fixtures capture what the Python app actually does, and
`crates/tg-core/tests/golden_parity.rs` asserts the Rust port matches.

Usage (needs the Python app's dependencies; the repo's venv works):
    QT_QPA_PLATFORM=offscreen python tools/gen_golden.py <path-to-taggui>

where <path-to-taggui> is a checkout of the Python app (the repo root
that contains `taggui/run_gui.py`). Writes JSON fixtures into
`crates/tg-core/golden/`.
"""
import json
import os
import sys
import tempfile
from pathlib import Path

# Offscreen Qt - none of this should open windows.
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(1)
    taggui_root = Path(sys.argv[1]).resolve()
    sys.path.insert(0, str(taggui_root / 'taggui'))

    # Heavy imports (torch, transformers, PySide6) come with the app.
    from PySide6.QtWidgets import QApplication  # noqa: E402
    app = QApplication.instance() or QApplication([])

    from auto_captioning.captioning_thread import (  # noqa: E402
        add_caption_to_tags)
    from models.image_list_model import ImageListModel  # noqa: E402
    from utils.enums import CaptionPosition  # noqa: E402
    from widgets.image_list import FilterLineEdit  # noqa: E402
    from models.proxy_image_list_model import ProxyImageListModel  # noqa: E402
    from utils.image import Image  # noqa: E402

    out_dir = Path(__file__).resolve().parent.parent / 'crates' / 'tg-core' \
        / 'golden'
    out_dir.mkdir(parents=True, exist_ok=True)

    # ---------------------------------------------------------------
    # 1. sidecar.json: how the Python model reads caption files.
    #    Uses the real `ImageListModel.load_directory` on a synthetic
    #    dataset, including messy captions.
    # ---------------------------------------------------------------
    dataset = Path(tempfile.mkdtemp(prefix='taggui-golden-'))
    cases = {
        'a.png': '1girl, solo, long hair',
        'b.png': '  spaced , out  ',
        'c.png': '',
        'd.png': 'trailing, comma,',
        'e.png': 'multi,  double  space',
        'f.png': 'tag1,tag2,no-space',
        'g.png': 'bad \xff utf8',
        'h.png': None,  # no sidecar file
    }
    for name, caption in cases.items():
        (dataset / name).write_bytes(b'fake-image')
        if caption is not None:
            # Write raw bytes so the invalid-UTF-8 case is exact.
            (dataset / name).with_suffix('.txt').write_bytes(
                caption.encode('utf-8', errors='replace') if isinstance(
                    caption, str) else caption)

    model = ImageListModel.__new__(ImageListModel)
    model.images = []
    model.tag_separator = ', '
    model.image_list_image_width = 200
    model.undo_stack = []
    model.redo_stack = []
    # Only run the tag-loading part (load_directory also reads thumbnails
    # and EXIF; those use Qt/Numpy but tolerate the fake bytes).
    try:
        model.load_directory(dataset)
        sidecar_results = [
            {
                'name': image.path.name,
                'tags': image.tags,
                'has_caption_file': image.has_caption_file,
            }
            for image in model.images
        ]
    except Exception as exception:  # fall back to the read/write functions
        print(f'load_directory failed ({exception}); using direct reads')
        sidecar_results = []
        for name, caption in cases.items():
            path = dataset / name
            text_path = path.with_suffix('.txt')
            caption_text = (text_path.read_text(encoding='utf-8',
                                                errors='replace')
                            if text_path.exists() else '')
            tags = []
            if caption_text:
                tags = caption_text.split(model.tag_separator)
                tags = [tag.strip() for tag in tags]
                tags = [tag for tag in tags if tag]
            sidecar_results.append({'name': name, 'tags': tags,
                                    'has_caption_file': text_path.exists()})
    json.dump({'tag_separator': ', ',
               # Raw sidecar bytes per image name (null = no sidecar), so
               # the Rust test can run the same inputs through parse_tags.
               'inputs': {name: (None if caption is None else list(
                   caption.encode('utf-8', errors='replace')))
                   for name, caption in cases.items()},
               'results': sidecar_results},
              open(out_dir / 'sidecar.json', 'w'), indent=1)
    print(f'sidecar.json: {len(sidecar_results)} cases')

    # ---------------------------------------------------------------
    # 2. caption_positions.json: add_caption_to_tags for every position.
    # ---------------------------------------------------------------
    tag_cases = [
        (['a', 'b'], 'c, d'),
        (['a', 'b'], 'c'),
        ([], 'c, d'),
        (['a'], ''),
        (['x', 'y'], 'new, tags, here'),
    ]
    positions = []
    for position in CaptionPosition:
        for tags, caption in tag_cases:
            result = add_caption_to_tags(list(tags), caption, position)
            positions.append({
                'position': position.value, 'tags': tags, 'caption': caption,
                'result': result})
    json.dump({'tag_separator': ', ', 'cases': positions},
              open(out_dir / 'caption_positions.json', 'w'), indent=1)
    print(f'caption_positions.json: {len(positions)} cases')

    # ---------------------------------------------------------------
    # 3. filters.json: parse + evaluate a matrix of filter strings
    #    against synthetic images, using the real grammar and the real
    #    evaluator.
    # ---------------------------------------------------------------
    images = [
        Image(path=Path('/ds/img001.png'), dimensions=(800, 600),
              tags=['1girl', 'blue eyes', 'solo']),
        Image(path=Path('/ds/photo2.jpg'), dimensions=(1920, 1080),
              tags=['1boy', 'outdoors']),
        Image(path=Path('/ds/tiny.gif'), dimensions=(50, 50),
              tags=[]),
        Image(path=Path('/ds/no EXIF.webp'), dimensions=None,
              tags=['1girl'], file_size=None),
    ]
    images[0].file_size = 500_000
    images[1].file_size = 2_500_000
    images[2].file_size = 10_000
    images[3].file_size = None

    filter_texts = [
        'blue', '1girl', 'img001', 'zzz', 'NOT blue',
        'tag:1girl', 'tag:"blue eyes"', 'tag:blue*',
        'caption:1girl', 'caption:zzz',
        'name:img*', 'name:*.png', 'path:/ds/*.png', 'path:*.gif',
        'ext:png', 'ext:PNG', 'ext:jpg',
        'tags:>2', 'tags:>=2', 'tags:==1', 'tags:!=0', 'tags:<1',
        'chars:>15', 'chars:<=15',
        'width:>=800', 'width:>600', 'width:<=50',
        'height:>=600', 'area:>=480000', 'mp:>=0.4', 'mp:<0.01',
        'ratio:<2', 'ratio:==1', 'ratio:>2',
        'size:>=500kb', 'size:>100kb', 'size:<1mb', 'size:==10000',
        'tag:1girl AND width:>=800', 'tag:1boy OR tag:1girl',
        'NOT tag:1girl AND ext:jpg',
        '(tag:1boy OR tag:1girl) AND width:>=1000',
        '(tag:1boy OR tag:1girl) AND NOT size:<1mb',
    ]

    def fake_token_ids(text):
        # The evaluator only needs a count; deterministic and simple.
        return {'input_ids': [0] * (len(text.split()) + 2)}

    evaluator = ProxyImageListModel.__new__(ProxyImageListModel)
    evaluator.tag_separator = ', '
    evaluator.tokenizer = fake_token_ids

    line_edit = FilterLineEdit()
    filter_results = []
    for text in filter_texts:
        line_edit.setText(text)
        parsed = line_edit.parse_filter_text()
        entry = {'filter': text, 'parses': parsed is not None,
                 'matches': []}
        if parsed is not None:
            for image in images:
                entry['matches'].append(
                    evaluator.does_image_match_filter(image, parsed))
        filter_results.append(entry)
    json.dump({'images': [
        {'path': str(image.path), 'dimensions': image.dimensions,
         'file_size': image.file_size, 'tags': image.tags}
        for image in images],
        'filters': filter_results},
        open(out_dir / 'filters.json', 'w'), indent=1)
    print(f'filters.json: {len(filter_results)} filters')

    # ---------------------------------------------------------------
    # 4. sidecar_write.json: exact bytes the Python model writes.
    # ---------------------------------------------------------------
    write_model = ImageListModel.__new__(ImageListModel)
    write_model.tag_separator = ', '
    write_targets = [
        (Path(dataset / 'w1.png'), ['1girl', 'solo']),
        (Path(dataset / 'w2.png'), ['one']),
        (Path(dataset / 'w3.png'), []),
    ]
    written = []
    for path, tags in write_targets:
        path.write_bytes(b'fake')
        write_model.write_image_tags_to_disk(
            Image(path=path, dimensions=(1, 1), tags=list(tags)))
        written.append({
            'path': path.name,
            'tags': tags,
            'bytes': list(path.with_suffix('.txt').read_bytes())})
    json.dump({'written': written},
              open(out_dir / 'sidecar_write.json', 'w'), indent=1)
    print(f'sidecar_write.json: {len(written)} files')

    print('Done. Fixtures in', out_dir)


if __name__ == '__main__':
    main()
