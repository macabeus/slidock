"""Converts the probe deck with fonts embedded: loads the flat ODP in a running LibreOffice, turns EmbedFonts on (a
flat file's own setting is not applied on load) and stores it as ODP and PPTX. Run inside the probe image by build.sh:
python3 convert.py <source.fodp> <out folder>."""

import os
import sys
import time

import uno
from com.sun.star.beans import PropertyValue


def prop(name, value):
    p = PropertyValue()
    p.Name = name
    p.Value = value
    return p


def connect():
    local = uno.getComponentContext()
    resolver = local.ServiceManager.createInstanceWithContext("com.sun.star.bridge.UnoUrlResolver", local)
    for _ in range(100):
        try:
            return resolver.resolve("uno:pipe,name=slidock-probe;urp;StarOffice.ComponentContext")
        except Exception:
            time.sleep(0.2)
    raise SystemExit("LibreOffice did not start")


def main(source, out):
    context = connect()
    desktop = context.ServiceManager.createInstanceWithContext("com.sun.star.frame.Desktop", context)
    doc = desktop.loadComponentFromURL(uno.systemPathToFileUrl(source), "_blank", 0, (prop("Hidden", True),))
    settings = doc.createInstance("com.sun.star.document.Settings")
    settings.EmbedFonts = True
    settings.EmbedOnlyUsedFonts = True
    settings.EmbedLatinScriptFonts = True
    settings.EmbedAsianScriptFonts = False
    settings.EmbedComplexScriptFonts = False
    stem = os.path.splitext(os.path.basename(source))[0]
    for ext, filter_name in (("odp", "impress8"), ("pptx", "Impress MS PowerPoint 2007 XML")):
        target = uno.systemPathToFileUrl(os.path.join(out, stem + "." + ext))
        doc.storeToURL(target, (prop("FilterName", filter_name), prop("Overwrite", True)))
    doc.close(True)
    try:
        desktop.terminate()
    except Exception:
        pass


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
