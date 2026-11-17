package dev.zorowarrior.bridge;

import net.minecraft.block.Block;
import net.minecraft.block.material.Material;
import net.minecraft.client.resources.model.ModelResourceLocation;
import net.minecraft.creativetab.CreativeTabs;
import net.minecraft.item.Item;
import net.minecraft.item.ItemBlock;
import net.minecraft.item.ItemStack;
import net.minecraftforge.client.model.ModelLoader;
import net.minecraftforge.fml.common.Mod;
import net.minecraftforge.fml.common.event.FMLInitializationEvent;
import net.minecraftforge.fml.common.event.FMLPreInitializationEvent;
import net.minecraftforge.fml.common.registry.GameRegistry;
import net.minecraftforge.fml.relauncher.Side;
import net.minecraftforge.fml.relauncher.SideOnly;

/**
 * ZOROWARRIOR HORIZON BRIDGE — Minecraft 1.8.9, Forge.
 *
 * <p>A genuinely different implementation, not a copy. 1.8.9 predates the
 * registry system: blocks and items are handed to {@code GameRegistry.register}
 * with a name, there is no {@code Block.Settings}, and models are registered at
 * runtime through {@code ModelLoader}. Nothing here compiles on 1.21.x and the
 * 1.21.x source does not compile here — which is why the jars are separate.
 *
 * <p>Details that differ from modern versions and are easy to get wrong:
 * <ul>
 *   <li>{@code ModelResourceLocation} is in {@code net.minecraft.client.resources.model}
 *       (it moved to {@code ...client.renderer.block.model} in 1.9).</li>
 *   <li>{@code Block.setHarvestLevel} returns {@code void}, so it cannot be
 *       chained onto the block builder.</li>
 *   <li>{@code GameRegistry.register} takes a name, not a {@code setRegistryName}.</li>
 *   <li>Recipes are code, not JSON — data packs did not exist in 1.8.9.</li>
 *   <li>Textures live in {@code textures/blocks} and {@code textures/items}.</li>
 * </ul>
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
        bridgeBeacon = new Block(Material.rock);
        bridgeBeacon.setHardness(3.5F);
        bridgeBeacon.setResistance(6.0F);
        bridgeBeacon.setLightLevel(12.0F / 15.0F);
        bridgeBeacon.setStepSound(Block.soundTypeStone);
        bridgeBeacon.setUnlocalizedName("bridge_beacon");
        bridgeBeacon.setCreativeTab(CreativeTabs.tabBlock);
        // Returns void in 1.8.9 — cannot be part of a fluent chain.
        bridgeBeacon.setHarvestLevel("pickaxe", 2);
        GameRegistry.register(bridgeBeacon, "bridge_beacon");

        bridgeChip = new Item();
        bridgeChip.setUnlocalizedName("bridge_chip");
        bridgeChip.setCreativeTab(CreativeTabs.tabMaterials);
        bridgeChip.setMaxStackSize(64);
        GameRegistry.register(bridgeChip, "bridge_chip");

        GameRegistry.register(new ItemBlock(bridgeBeacon), "bridge_beacon");

        // 1.8.9 recipes: position strings plus a key, resolved at load time.
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
     * 1.8.9 resolves models from {@code blockstates/} and {@code models/} via the
     * resource pack, but the item-to-model binding for an ItemBlock still has to
     * be declared in code.
     */
    @SideOnly(Side.CLIENT)
    @Mod.EventHandler
    public void init(FMLInitializationEvent event) {
        ModelLoader.setCustomModelResourceLocation(
                Item.getItemFromBlock(bridgeBeacon), 0,
                new ModelResourceLocation(MODID + ":bridge_beacon", "inventory"));
        ModelLoader.setCustomModelResourceLocation(
                bridgeChip, 0,
                new ModelResourceLocation(MODID + ":bridge_chip", "inventory"));
    }
}
