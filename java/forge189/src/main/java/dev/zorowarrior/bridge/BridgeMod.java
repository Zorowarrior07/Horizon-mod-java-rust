package dev.zorowarrior.bridge;

import net.minecraft.block.Block;
import net.minecraft.block.material.Material;
import net.minecraft.client.renderer.block.model.ModelResourceLocation;
import net.minecraft.creativetab.CreativeTabs;
import net.minecraft.item.Item;
import net.minecraft.item.ItemBlock;
import net.minecraft.item.ItemStack;
import net.minecraft.util.ResourceLocation;
import net.minecraftforge.client.model.ModelLoader;
import net.minecraftforge.fml.common.Mod;
import net.minecraftforge.fml.common.event.FMLPreInitializationEvent;
import net.minecraftforge.fml.common.registry.GameRegistry;
import net.minecraftforge.fml.relauncher.Side;
import net.minecraftforge.fml.relauncher.SideOnly;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Minecraft 1.8.9, Forge.
 *
 * <p>This is a genuinely different implementation, not a copy: 1.8.9 predates
 * the registry system entirely. Blocks and items are registered through
 * {@code GameRegistry.register} with a {@code setRegistryName}, there are no
 * {@code Block.Settings}, and models are registered per-item at runtime through
 * {@code ModelLoader}. Nothing here compiles on 1.21.x, and the 1.21.x code does
 * not compile here — which is exactly why they are separate jars.
 *
 * <p>1.8.9 Forge also requires a Java 8 class-file level, enforced by the build.
 */
@Mod(modid = BridgeMod.MODID, version = BridgeMod.VERSION,
     name = "ZOROWARRIOR HORIZON BRIDGE",
     acceptedMinecraftVersions = "[1.8.9]")
public class BridgeMod {

    public static final String MODID = "zoro_bridge";
    public static final String VERSION = "1.0.0";

    public static Block bridgeBeacon;
    public static Item bridgeChip;

    @Mod.EventHandler
    public void preInit(FMLPreInitializationEvent event) {
        // 1.8.9: Material + setHardness/setResistance/setLightLevel, no builder.
        bridgeBeacon = new Block(Material.rock)
                .setHardness(3.5F)
                .setResistance(6.0F)
                .setLightLevel(12.0F / 15.0F)
                .setStepSound(Block.soundTypeStone)
                .setHarvestLevel("pickaxe", 2)
                .setUnlocalizedName("bridge_beacon")
                .setCreativeTab(CreativeTabs.tabBlock);
        bridgeBeacon.setRegistryName(new ResourceLocation(MODID, "bridge_beacon"));
        GameRegistry.register(bridgeBeacon);

        bridgeChip = new Item()
                .setUnlocalizedName("bridge_chip")
                .setCreativeTab(CreativeTabs.tabMaterials)
                .setMaxStackSize(64);
        bridgeChip.setRegistryName(new ResourceLocation(MODID, "bridge_chip"));
        GameRegistry.register(bridgeChip);

        GameRegistry.register(new ItemBlock(bridgeBeacon)
                .setRegistryName(bridgeBeacon.getRegistryName()));

        // Recipes in 1.8.9 are code, not JSON: data packs did not exist yet.
        GameRegistry.addRecipe(new ItemStack(bridgeChip),
                " G ",
                "GDG",
                " G ",
                'G', new ItemStack(net.minecraft.init.Items.gold_ingot),
                'D', new ItemStack(net.minecraft.init.Items.diamond));
        GameRegistry.addRecipe(new ItemStack(bridgeBeacon),
                "SSS",
                "SCS",
                "SSS",
                'S', new ItemStack(net.minecraft.init.Blocks.stone),
                'C', new ItemStack(bridgeChip));
    }

    /**
     * 1.8.9 loads models from the classpath at runtime, so each item needs an
     * explicit model registration — there is no "items/<name>.json" convention.
     */
    @SideOnly(Side.CLIENT)
    @Mod.EventHandler
    public void initModels(net.minecraftforge.fml.common.event.FMLInitializationEvent event) {
        ModelLoader.setCustomModelResourceLocation(
                Item.getItemFromBlock(bridgeBeacon), 0,
                new ModelResourceLocation(MODID + ":bridge_beacon", "inventory"));
        ModelLoader.setCustomModelResourceLocation(
                bridgeChip, 0,
                new ModelResourceLocation(MODID + ":bridge_chip", "inventory"));
    }
}
