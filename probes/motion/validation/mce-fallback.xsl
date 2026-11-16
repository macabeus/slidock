<?xml version="1.0" encoding="UTF-8"?>
<!-- Markup Compatibility (ECMA-376 Part 3) as a consumer that understands no extension namespace: every
     mc:AlternateContent is replaced by its mc:Fallback content and mc:Ignorable is dropped, so the part can be
     validated against the ECMA-376 transitional schemas. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
  xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
  <xsl:template match="@*|node()"><xsl:copy><xsl:apply-templates select="@*|node()"/></xsl:copy></xsl:template>
  <xsl:template match="mc:AlternateContent"><xsl:apply-templates select="mc:Fallback/node()"/></xsl:template>
  <xsl:template match="@mc:Ignorable"/>
</xsl:stylesheet>
