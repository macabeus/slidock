<?xml version="1.0" encoding="UTF-8"?>
<!-- Extracts every mc:Choice of a part (one element per line) so the extension markup can at least be checked for
     well-formedness and namespaces; no public XSD covers p14/p15/p159. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
  xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
  <xsl:output method="xml" omit-xml-declaration="yes"/>
  <xsl:template match="/"><choices><xsl:copy-of select="//mc:Choice"/></choices></xsl:template>
</xsl:stylesheet>
